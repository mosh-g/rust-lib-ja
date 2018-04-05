// Copyright 2014-2016 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Basic syntax highlighting functionality.
//!
//! This module uses libsyntax's lexer to provide token-based highlighting for
//! the HTML documentation generated by rustdoc.
//!
//! If you just want to syntax highlighting for a Rust program, then you can use
//! the `render_inner_with_highlighting` or `render_with_highlighting`
//! functions. For more advanced use cases (if you want to supply your own css
//! classes or control how the HTML is generated, or even generate something
//! other then HTML), then you should implement the `Writer` trait and use a
//! `Classifier`.

use html::escape::Escape;

use std::fmt::Display;
use std::io;
use std::io::prelude::*;

use syntax::codemap::{CodeMap, FilePathMapping};
use syntax::parse::lexer::{self, TokenAndSpan};
use syntax::parse::token;
use syntax::parse;
use syntax_pos::{Span, FileName};

/// Highlights `src`, returning the HTML output.
pub fn render_with_highlighting(src: &str, class: Option<&str>, id: Option<&str>,
                                extension: Option<&str>,
                                tooltip: Option<(&str, &str)>) -> String {
    debug!("highlighting: ================\n{}\n==============", src);
    let sess = parse::ParseSess::new(FilePathMapping::empty());
    let fm = sess.codemap().new_filemap(FileName::Custom("stdin".to_string()), src.to_string());

    let mut out = Vec::new();
    if let Some((tooltip, class)) = tooltip {
        write!(out, "<div class='information'><div class='tooltip {}'>ⓘ<span \
                     class='tooltiptext'>{}</span></div></div>",
               class, tooltip).unwrap();
    }
    write_header(class, id, &mut out).unwrap();

    let mut classifier = Classifier::new(lexer::StringReader::new(&sess, fm), sess.codemap());
    if let Err(_) = classifier.write_source(&mut out) {
        return format!("<pre>{}</pre>", src);
    }

    if let Some(extension) = extension {
        write!(out, "{}", extension).unwrap();
    }
    write_footer(&mut out).unwrap();
    String::from_utf8_lossy(&out[..]).into_owned()
}

/// Highlights `src`, returning the HTML output. Returns only the inner html to
/// be inserted into an element. C.f., `render_with_highlighting` which includes
/// an enclosing `<pre>` block.
pub fn render_inner_with_highlighting(src: &str) -> io::Result<String> {
    let sess = parse::ParseSess::new(FilePathMapping::empty());
    let fm = sess.codemap().new_filemap(FileName::Custom("stdin".to_string()), src.to_string());

    let mut out = Vec::new();
    let mut classifier = Classifier::new(lexer::StringReader::new(&sess, fm), sess.codemap());
    classifier.write_source(&mut out)?;

    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// Processes a program (nested in the internal `lexer`), classifying strings of
/// text by highlighting category (`Class`). Calls out to a `Writer` to write
/// each span of text in sequence.
pub struct Classifier<'a> {
    lexer: lexer::StringReader<'a>,
    codemap: &'a CodeMap,

    // State of the classifier.
    in_attribute: bool,
    in_macro: bool,
    in_macro_nonterminal: bool,
}

/// How a span of text is classified. Mostly corresponds to token kinds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Class {
    None,
    Comment,
    DocComment,
    Attribute,
    KeyWord,
    // Keywords that do pointer/reference stuff.
    RefKeyWord,
    Self_,
    Op,
    Macro,
    MacroNonTerminal,
    String,
    Number,
    Bool,
    Ident,
    Lifetime,
    PreludeTy,
    PreludeVal,
    QuestionMark,
}

/// Trait that controls writing the output of syntax highlighting. Users should
/// implement this trait to customize writing output.
///
/// The classifier will call into the `Writer` implementation as it finds spans
/// of text to highlight. Exactly how that text should be highlighted is up to
/// the implementation.
pub trait Writer {
    /// Called when we start processing a span of text that should be highlighted.
    /// The `Class` argument specifies how it should be highlighted.
    fn enter_span(&mut self, _: Class) -> io::Result<()>;

    /// Called at the end of a span of highlighted text.
    fn exit_span(&mut self) -> io::Result<()>;

    /// Called for a span of text, usually, but not always, a single token. If
    /// the string of text (`T`) does correspond to a token, then the token will
    /// also be passed. If the text should be highlighted differently from the
    /// surrounding text, then the `Class` argument will be a value other than
    /// `None`.
    /// The following sequences of callbacks are equivalent:
    /// ```plain
    ///     enter_span(Foo), string("text", None), exit_span()
    ///     string("text", Foo)
    /// ```
    /// The latter can be thought of as a shorthand for the former, which is
    /// more flexible.
    fn string<T: Display>(&mut self,
                          text: T,
                          klass: Class,
                          tok: Option<&TokenAndSpan>)
                          -> io::Result<()>;
}

// Implement `Writer` for anthing that can be written to, this just implements
// the default rustdoc behaviour.
impl<U: Write> Writer for U {
    fn string<T: Display>(&mut self,
                          text: T,
                          klass: Class,
                          _tas: Option<&TokenAndSpan>)
                          -> io::Result<()> {
        match klass {
            Class::None => write!(self, "{}", text),
            klass => write!(self, "<span class=\"{}\">{}</span>", klass.rustdoc_class(), text),
        }
    }

    fn enter_span(&mut self, klass: Class) -> io::Result<()> {
        write!(self, "<span class=\"{}\">", klass.rustdoc_class())
    }

    fn exit_span(&mut self) -> io::Result<()> {
        write!(self, "</span>")
    }
}

impl<'a> Classifier<'a> {
    pub fn new(lexer: lexer::StringReader<'a>, codemap: &'a CodeMap) -> Classifier<'a> {
        Classifier {
            lexer,
            codemap,
            in_attribute: false,
            in_macro: false,
            in_macro_nonterminal: false,
        }
    }

    /// Gets the next token out of the lexer, emitting fatal errors if lexing fails.
    fn try_next_token(&mut self) -> io::Result<TokenAndSpan> {
        match self.lexer.try_next_token() {
            Ok(tas) => Ok(tas),
            Err(_) => {
                self.lexer.emit_fatal_errors();
                self.lexer.sess.span_diagnostic
                    .struct_warn("Backing out of syntax highlighting")
                    .note("You probably did not intend to render this as a rust code-block")
                    .emit();
                Err(io::Error::new(io::ErrorKind::Other, ""))
            }
        }
    }

    /// Exhausts the `lexer` writing the output into `out`.
    ///
    /// The general structure for this method is to iterate over each token,
    /// possibly giving it an HTML span with a class specifying what flavor of token
    /// is used. All source code emission is done as slices from the source map,
    /// not from the tokens themselves, in order to stay true to the original
    /// source.
    pub fn write_source<W: Writer>(&mut self,
                                   out: &mut W)
                                   -> io::Result<()> {
        loop {
            let next = self.try_next_token()?;
            if next.tok == token::Eof {
                break;
            }

            self.write_token(out, next)?;
        }

        Ok(())
    }

    // Handles an individual token from the lexer.
    fn write_token<W: Writer>(&mut self,
                              out: &mut W,
                              tas: TokenAndSpan)
                              -> io::Result<()> {
        let klass = match tas.tok {
            token::Shebang(s) => {
                out.string(Escape(&s.as_str()), Class::None, Some(&tas))?;
                return Ok(());
            },

            token::Whitespace => Class::None,
            token::Comment => Class::Comment,
            token::DocComment(..) => Class::DocComment,

            // If this '&' or '*' token is followed by a non-whitespace token, assume that it's the
            // reference or dereference operator or a reference or pointer type, instead of the
            // bit-and or multiplication operator.
            token::BinOp(token::And) | token::BinOp(token::Star)
                if self.lexer.peek().tok != token::Whitespace => Class::RefKeyWord,

            // Consider this as part of a macro invocation if there was a
            // leading identifier.
            token::Not if self.in_macro => {
                self.in_macro = false;
                Class::Macro
            }

            // Operators.
            token::Eq | token::Lt | token::Le | token::EqEq | token::Ne | token::Ge | token::Gt |
                token::AndAnd | token::OrOr | token::Not | token::BinOp(..) | token::RArrow |
                token::BinOpEq(..) | token::FatArrow => Class::Op,

            // Miscellaneous, no highlighting.
            token::Dot | token::DotDot | token::DotDotDot | token::DotDotEq | token::Comma |
                token::Semi | token::Colon | token::ModSep | token::LArrow | token::OpenDelim(_) |
                token::CloseDelim(token::Brace) | token::CloseDelim(token::Paren) |
                token::CloseDelim(token::NoDelim) => Class::None,

            token::Question => Class::QuestionMark,

            token::Dollar => {
                if self.lexer.peek().tok.is_ident() {
                    self.in_macro_nonterminal = true;
                    Class::MacroNonTerminal
                } else {
                    Class::None
                }
            }

            // This might be the start of an attribute. We're going to want to
            // continue highlighting it as an attribute until the ending ']' is
            // seen, so skip out early. Down below we terminate the attribute
            // span when we see the ']'.
            token::Pound => {
                // We can't be sure that our # begins an attribute (it could
                // just be appearing in a macro) until we read either `#![` or
                // `#[` from the input stream.
                //
                // We don't want to start highlighting as an attribute until
                // we're confident there is going to be a ] coming up, as
                // otherwise # tokens in macros highlight the rest of the input
                // as an attribute.

                // Case 1: #![inner_attribute]
                if self.lexer.peek().tok == token::Not {
                    self.try_next_token()?; // NOTE: consumes `!` token!
                    if self.lexer.peek().tok == token::OpenDelim(token::Bracket) {
                        self.in_attribute = true;
                        out.enter_span(Class::Attribute)?;
                    }
                    out.string("#", Class::None, None)?;
                    out.string("!", Class::None, None)?;
                    return Ok(());
                }

                // Case 2: #[outer_attribute]
                if self.lexer.peek().tok == token::OpenDelim(token::Bracket) {
                    self.in_attribute = true;
                    out.enter_span(Class::Attribute)?;
                }
                out.string("#", Class::None, None)?;
                return Ok(());
            }
            token::CloseDelim(token::Bracket) => {
                if self.in_attribute {
                    self.in_attribute = false;
                    out.string("]", Class::None, None)?;
                    out.exit_span()?;
                    return Ok(());
                } else {
                    Class::None
                }
            }

            token::Literal(lit, _suf) => {
                match lit {
                    // Text literals.
                    token::Byte(..) | token::Char(..) |
                        token::ByteStr(..) | token::ByteStrRaw(..) |
                        token::Str_(..) | token::StrRaw(..) => Class::String,

                    // Number literals.
                    token::Integer(..) | token::Float(..) => Class::Number,
                }
            }

            // Keywords are also included in the identifier set.
            token::Ident(ident, is_raw) => {
                match &*ident.name.as_str() {
                    "ref" | "mut" if !is_raw => Class::RefKeyWord,

                    "self" | "Self" => Class::Self_,
                    "false" | "true" if !is_raw => Class::Bool,

                    "Option" | "Result" => Class::PreludeTy,
                    "Some" | "None" | "Ok" | "Err" => Class::PreludeVal,

                    "$crate" => Class::KeyWord,
                    _ if tas.tok.is_reserved_ident() => Class::KeyWord,

                    _ => {
                        if self.in_macro_nonterminal {
                            self.in_macro_nonterminal = false;
                            Class::MacroNonTerminal
                        } else if self.lexer.peek().tok == token::Not {
                            self.in_macro = true;
                            Class::Macro
                        } else {
                            Class::Ident
                        }
                    }
                }
            }

            token::Lifetime(..) => Class::Lifetime,

            token::Eof | token::Interpolated(..) |
            token::Tilde | token::At | token::DotEq => Class::None,
        };

        // Anything that didn't return above is the simple case where we the
        // class just spans a single token, so we can use the `string` method.
        out.string(Escape(&self.snip(tas.sp)), klass, Some(&tas))
    }

    // Helper function to get a snippet from the codemap.
    fn snip(&self, sp: Span) -> String {
        self.codemap.span_to_snippet(sp).unwrap()
    }
}

impl Class {
    /// Returns the css class expected by rustdoc for each `Class`.
    pub fn rustdoc_class(self) -> &'static str {
        match self {
            Class::None => "",
            Class::Comment => "comment",
            Class::DocComment => "doccomment",
            Class::Attribute => "attribute",
            Class::KeyWord => "kw",
            Class::RefKeyWord => "kw-2",
            Class::Self_ => "self",
            Class::Op => "op",
            Class::Macro => "macro",
            Class::MacroNonTerminal => "macro-nonterminal",
            Class::String => "string",
            Class::Number => "number",
            Class::Bool => "bool-val",
            Class::Ident => "ident",
            Class::Lifetime => "lifetime",
            Class::PreludeTy => "prelude-ty",
            Class::PreludeVal => "prelude-val",
            Class::QuestionMark => "question-mark"
        }
    }
}

fn write_header(class: Option<&str>,
                id: Option<&str>,
                out: &mut Write)
                -> io::Result<()> {
    write!(out, "<pre ")?;
    if let Some(id) = id {
        write!(out, "id='{}' ", id)?;
    }
    write!(out, "class=\"rust {}\">\n", class.unwrap_or(""))
}

fn write_footer(out: &mut Write) -> io::Result<()> {
    write!(out, "</pre>\n")
}
