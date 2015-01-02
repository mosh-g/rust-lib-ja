// Copyright 2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use ast::{TokenTree, TtDelimited, TtSequence, TtToken};
use ast;
use codemap::{Span, DUMMY_SP};
use ext::base::{ExtCtxt, MacResult, SyntaxExtension};
use ext::base::{NormalTT, TTMacroExpander};
use ext::tt::macro_parser::{Success, Error, Failure};
use ext::tt::macro_parser::{NamedMatch, MatchedSeq, MatchedNonterminal};
use ext::tt::macro_parser::{parse, parse_or_else};
use parse::lexer::new_tt_reader;
use parse::parser::Parser;
use parse::attr::ParserAttr;
use parse::token::{special_idents, gensym_ident, NtTT, Token};
use parse::token::Token::*;
use parse::token;
use print;
use ptr::P;

use util::small_vector::SmallVector;

use std::cell::RefCell;
use std::rc::Rc;

struct ParserAnyMacro<'a> {
    parser: RefCell<Parser<'a>>,
}

impl<'a> ParserAnyMacro<'a> {
    /// Make sure we don't have any tokens left to parse, so we don't
    /// silently drop anything. `allow_semi` is so that "optional"
    /// semicolons at the end of normal expressions aren't complained
    /// about e.g. the semicolon in `macro_rules! kapow { () => {
    /// panic!(); } }` doesn't get picked up by .parse_expr(), but it's
    /// allowed to be there.
    fn ensure_complete_parse(&self, allow_semi: bool) {
        let mut parser = self.parser.borrow_mut();
        if allow_semi && parser.token == token::Semi {
            parser.bump()
        }
        if parser.token != token::Eof {
            let token_str = parser.this_token_to_string();
            let msg = format!("macro expansion ignores token `{}` and any \
                               following",
                              token_str);
            let span = parser.span;
            parser.span_err(span, msg[]);
        }
    }
}

impl<'a> MacResult for ParserAnyMacro<'a> {
    fn make_expr(self: Box<ParserAnyMacro<'a>>) -> Option<P<ast::Expr>> {
        let ret = self.parser.borrow_mut().parse_expr();
        self.ensure_complete_parse(true);
        Some(ret)
    }
    fn make_pat(self: Box<ParserAnyMacro<'a>>) -> Option<P<ast::Pat>> {
        let ret = self.parser.borrow_mut().parse_pat();
        self.ensure_complete_parse(false);
        Some(ret)
    }
    fn make_items(self: Box<ParserAnyMacro<'a>>) -> Option<SmallVector<P<ast::Item>>> {
        let mut ret = SmallVector::zero();
        loop {
            let mut parser = self.parser.borrow_mut();
            // so... do outer attributes attached to the macro invocation
            // just disappear? This question applies to make_methods, as
            // well.
            match parser.parse_item_with_outer_attributes() {
                Some(item) => ret.push(item),
                None => break
            }
        }
        self.ensure_complete_parse(false);
        Some(ret)
    }

    fn make_methods(self: Box<ParserAnyMacro<'a>>) -> Option<SmallVector<P<ast::Method>>> {
        let mut ret = SmallVector::zero();
        loop {
            let mut parser = self.parser.borrow_mut();
            match parser.token {
                token::Eof => break,
                _ => {
                    let attrs = parser.parse_outer_attributes();
                    ret.push(parser.parse_method(attrs, ast::Inherited))
                }
            }
        }
        self.ensure_complete_parse(false);
        Some(ret)
    }

    fn make_stmt(self: Box<ParserAnyMacro<'a>>) -> Option<P<ast::Stmt>> {
        let attrs = self.parser.borrow_mut().parse_outer_attributes();
        let ret = self.parser.borrow_mut().parse_stmt(attrs);
        self.ensure_complete_parse(true);
        Some(ret)
    }
}

struct MacroRulesMacroExpander {
    name: ast::Ident,
    imported_from: Option<ast::Ident>,
    lhses: Vec<Rc<NamedMatch>>,
    rhses: Vec<Rc<NamedMatch>>,
}

impl TTMacroExpander for MacroRulesMacroExpander {
    fn expand<'cx>(&self,
                   cx: &'cx mut ExtCtxt,
                   sp: Span,
                   arg: &[ast::TokenTree])
                   -> Box<MacResult+'cx> {
        generic_extension(cx,
                          sp,
                          self.name,
                          self.imported_from,
                          arg,
                          self.lhses[],
                          self.rhses[])
    }
}

/// Given `lhses` and `rhses`, this is the new macro we create
fn generic_extension<'cx>(cx: &'cx ExtCtxt,
                          sp: Span,
                          name: ast::Ident,
                          imported_from: Option<ast::Ident>,
                          arg: &[ast::TokenTree],
                          lhses: &[Rc<NamedMatch>],
                          rhses: &[Rc<NamedMatch>])
                          -> Box<MacResult+'cx> {
    if cx.trace_macros() {
        println!("{}! {{ {} }}",
                 token::get_ident(name),
                 print::pprust::tts_to_string(arg));
    }

    // Which arm's failure should we report? (the one furthest along)
    let mut best_fail_spot = DUMMY_SP;
    let mut best_fail_msg = "internal error: ran no matchers".to_string();

    for (i, lhs) in lhses.iter().enumerate() { // try each arm's matchers
        match **lhs {
          MatchedNonterminal(NtTT(ref lhs_tt)) => {
            let lhs_tt = match **lhs_tt {
                TtDelimited(_, ref delim) => delim.tts[],
                _ => cx.span_fatal(sp, "malformed macro lhs")
            };
            // `None` is because we're not interpolating
            let mut arg_rdr = new_tt_reader(&cx.parse_sess().span_diagnostic,
                                            None,
                                            None,
                                            arg.iter()
                                               .map(|x| (*x).clone())
                                               .collect());
            arg_rdr.desugar_doc_comments = true;
            match parse(cx.parse_sess(), cx.cfg(), arg_rdr, lhs_tt) {
              Success(named_matches) => {
                let rhs = match *rhses[i] {
                    // okay, what's your transcriber?
                    MatchedNonterminal(NtTT(ref tt)) => {
                        match **tt {
                            // ignore delimiters
                            TtDelimited(_, ref delimed) => delimed.tts.clone(),
                            _ => cx.span_fatal(sp, "macro rhs must be delimited"),
                        }
                    },
                    _ => cx.span_bug(sp, "bad thing in rhs")
                };
                // rhs has holes ( `$id` and `$(...)` that need filled)
                let trncbr = new_tt_reader(&cx.parse_sess().span_diagnostic,
                                           Some(named_matches),
                                           imported_from,
                                           rhs);
                let p = Parser::new(cx.parse_sess(), cx.cfg(), box trncbr);
                // Let the context choose how to interpret the result.
                // Weird, but useful for X-macros.
                return box ParserAnyMacro {
                    parser: RefCell::new(p),
                } as Box<MacResult+'cx>
              }
              Failure(sp, ref msg) => if sp.lo >= best_fail_spot.lo {
                best_fail_spot = sp;
                best_fail_msg = (*msg).clone();
              },
              Error(sp, ref msg) => cx.span_fatal(sp, msg[])
            }
          }
          _ => cx.bug("non-matcher found in parsed lhses")
        }
    }
    cx.span_fatal(best_fail_spot, best_fail_msg[]);
}

// Note that macro-by-example's input is also matched against a token tree:
//                   $( $lhs:tt => $rhs:tt );+
//
// Holy self-referential!

/// Converts a `macro_rules!` invocation into a syntax extension.
pub fn compile<'cx>(cx: &'cx mut ExtCtxt,
                    def: &ast::MacroDef) -> SyntaxExtension {

    let lhs_nm =  gensym_ident("lhs");
    let rhs_nm =  gensym_ident("rhs");

    // The pattern that macro_rules matches.
    // The grammar for macro_rules! is:
    // $( $lhs:tt => $rhs:tt );+
    // ...quasiquoting this would be nice.
    // These spans won't matter, anyways
    let match_lhs_tok = MatchNt(lhs_nm, special_idents::tt, token::Plain, token::Plain);
    let match_rhs_tok = MatchNt(rhs_nm, special_idents::tt, token::Plain, token::Plain);
    let argument_gram = vec!(
        TtSequence(DUMMY_SP,
                   Rc::new(ast::SequenceRepetition {
                       tts: vec![
                           TtToken(DUMMY_SP, match_lhs_tok),
                           TtToken(DUMMY_SP, token::FatArrow),
                           TtToken(DUMMY_SP, match_rhs_tok)],
                       separator: Some(token::Semi),
                       op: ast::OneOrMore,
                       num_captures: 2
                   })),
        //to phase into semicolon-termination instead of
        //semicolon-separation
        TtSequence(DUMMY_SP,
                   Rc::new(ast::SequenceRepetition {
                       tts: vec![TtToken(DUMMY_SP, token::Semi)],
                       separator: None,
                       op: ast::ZeroOrMore,
                       num_captures: 0
                   })));


    // Parse the macro_rules! invocation (`none` is for no interpolations):
    let arg_reader = new_tt_reader(&cx.parse_sess().span_diagnostic,
                                   None,
                                   None,
                                   def.body.clone());
    let argument_map = parse_or_else(cx.parse_sess(),
                                     cx.cfg(),
                                     arg_reader,
                                     argument_gram);

    // Extract the arguments:
    let lhses = match *argument_map[lhs_nm] {
        MatchedSeq(ref s, _) => /* FIXME (#2543) */ (*s).clone(),
        _ => cx.span_bug(def.span, "wrong-structured lhs")
    };

    for lhs in lhses.iter() {
        check_lhs_nt_follows(cx, &**lhs, def.span);
    }

    let rhses = match *argument_map[rhs_nm] {
        MatchedSeq(ref s, _) => /* FIXME (#2543) */ (*s).clone(),
        _ => cx.span_bug(def.span, "wrong-structured rhs")
    };

    let exp = box MacroRulesMacroExpander {
        name: def.ident,
        imported_from: def.imported_from,
        lhses: lhses,
        rhses: rhses,
    };

    NormalTT(exp, Some(def.span))
}

fn check_lhs_nt_follows(cx: &mut ExtCtxt, lhs: &NamedMatch, sp: Span) {
    // lhs is going to be like MatchedNonterminal(NtTT(TtDelimited(...))), where
    // the entire lhs is those tts.
    // if ever we get box/deref patterns, this could turn into an `if let
    // &MatchedNonterminal(NtTT(box TtDelimited(...))) = lhs`
    let matcher = match lhs {
        &MatchedNonterminal(NtTT(ref inner)) => match &**inner {
            &TtDelimited(_, ref tts) => tts.tts[],
            _ => cx.span_bug(sp, "wrong-structured lhs for follow check")
        },
        _ => cx.span_bug(sp, "wrong-structured lhs for follow check")
    };

    check_matcher(cx, matcher, &Eof);
    // we don't abort on errors on rejection, the driver will do that for us
    // after parsing/expansion. we can report every error in every macro this way.
}

fn check_matcher(cx: &mut ExtCtxt, matcher: &[TokenTree], follow: &Token) {
    use print::pprust::token_to_string;

    // 1. If there are no tokens in M, accept
    if matcher.is_empty() {
        return;
    }

    // 2. For each token T in M:
    let mut tokens = matcher.iter().peekable();
    while let Some(token) = tokens.next() {
        match *token {
            TtToken(sp, MatchNt(ref name, ref frag_spec, _, _)) => {
                // ii. If T is a simple NT, look ahead to the next token T' in
                // M.
                let next_token = match tokens.peek() {
                    // If T' closes a complex NT, replace T' with F
                    Some(&&TtToken(_, CloseDelim(_))) => follow,
                    Some(&&TtToken(_, ref tok)) => tok,
                    // T' is any NT (this catches complex NTs, the next
                    // iteration will die if it's a TtDelimited).
                    Some(_) => continue,
                    // else, we're at the end of the macro or sequence
                    None => follow
                };

                // If T' is in the set FOLLOW(NT), continue. Else, reject.
                match *next_token {
                    Eof | MatchNt(..) => continue,
                    _ if is_in_follow(cx, next_token, frag_spec.as_str()) => continue,
                    ref tok => cx.span_err(sp, format!("`${0}:{1}` is followed by `{2}`, which \
                                                        is not allowed for `{1}` fragments",
                                                        name.as_str(), frag_spec.as_str(),
                                                        token_to_string(tok))[])
                }
            },
            TtSequence(_, ref seq) => {
                // iii. Else, T is a complex NT.
                match seq.separator {
                    // If T has the form $(...)U+ or $(...)U* for some token U,
                    // run the algorithm on the contents with F set to U. If it
                    // accepts, continue, else, reject.
                    Some(ref u) => check_matcher(cx, seq.tts[], u),
                    // If T has the form $(...)+ or $(...)*, run the algorithm
                    // on the contents with F set to EOF. If it accepts,
                    // continue, else, reject.
                    None => check_matcher(cx, seq.tts[], &Eof)
                }
            },
            TtToken(..) => {
                // i. If T is not an NT, continue.
                continue
            },
            TtDelimited(_, ref tts) => {
                // if we don't pass in that close delimiter, we'll incorrectly consider the matcher
                // `{ $foo:ty }` as having a follow that isn't `}`
                check_matcher(cx, tts.tts[], &tts.close_token())
            }
        }
    }
}

fn is_in_follow(cx: &ExtCtxt, tok: &Token, frag: &str) -> bool {
    if let &CloseDelim(_) = tok {
        return true;
    }

    match frag {
        "item" => {
            // since items *must* be followed by either a `;` or a `}`, we can
            // accept anything after them
            true
        },
        "block" => {
            // anything can follow block, the braces provide a easy boundary to
            // maintain
            true
        },
        "stmt" | "expr"  => {
            match *tok {
                Comma | Semi => true,
                _ => false
            }
        },
        "pat" => {
            match *tok {
                FatArrow | Comma | Eq => true,
                _ => false
            }
        },
        "path" | "ty" => {
            match *tok {
                Comma | RArrow | Colon | Eq | Gt => true,
                Ident(i, _) if i.as_str() == "as" => true,
                _ => false
            }
        },
        "ident" => {
            // being a single token, idents are harmless
            true
        },
        "meta" | "tt" => {
            // being either a single token or a delimited sequence, tt is
            // harmless
            true
        },
        _ => cx.bug(format!("unrecognized builtin nonterminal {}", frag)[]),
    }
}
