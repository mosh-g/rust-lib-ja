//! Types/fns concerning URLs (see RFC 3986)

import map;
import map::{hashmap, str_hash};
import io::Reader;
import dvec::{DVec, dvec};

export url, userinfo, query;
export from_str, to_str;
export get_scheme;

export encode, decode;
export encode_component, decode_component;
export encode_form_urlencoded, decode_form_urlencoded;

type url = {
    scheme: ~str,
    user: option<userinfo>,
    host: ~str,
    port: option<~str>,
    path: ~str,
    query: query,
    fragment: option<~str>
};

type userinfo = {
    user: ~str,
    pass: option<~str>
};

type query = ~[(~str, ~str)];

fn url(-scheme: ~str, -user: option<userinfo>, -host: ~str,
       -port: option<~str>, -path: ~str, -query: query,
       -fragment: option<~str>) -> url {
    { scheme: scheme, user: user, host: host, port: port,
     path: path, query: query, fragment: fragment }
}

fn userinfo(-user: ~str, -pass: option<~str>) -> userinfo {
    {user: user, pass: pass}
}

fn encode_inner(s: ~str, full_url: bool) -> ~str {
    do io::with_str_reader(s) |rdr| {
        let mut out = ~"";

        while !rdr.eof() {
            let ch = rdr.read_byte() as char;
            match ch {
              // unreserved:
              'A' to 'Z' |
              'a' to 'z' |
              '0' to '9' |
              '-' | '.' | '_' | '~' => {
                str::push_char(out, ch);
              }
              _ => {
                  if full_url {
                    match ch {
                      // gen-delims:
                      ':' | '/' | '?' | '#' | '[' | ']' | '@' |

                      // sub-delims:
                      '!' | '$' | '&' | '"' | '(' | ')' | '*' |
                      '+' | ',' | ';' | '=' => {
                        str::push_char(out, ch);
                      }

                      _ => out += #fmt("%%%X", ch as uint)
                    }
                } else {
                    out += #fmt("%%%X", ch as uint);
                }
              }
            }
        }

        out
    }
}

/**
 * Encodes a URI by replacing reserved characters with percent encoded
 * character sequences.
 *
 * This function is compliant with RFC 3986.
 */
fn encode(s: ~str) -> ~str {
    encode_inner(s, true)
}

/**
 * Encodes a URI component by replacing reserved characters with percent
 * encoded character sequences.
 *
 * This function is compliant with RFC 3986.
 */
fn encode_component(s: ~str) -> ~str {
    encode_inner(s, false)
}

fn decode_inner(s: ~str, full_url: bool) -> ~str {
    do io::with_str_reader(s) |rdr| {
        let mut out = ~"";

        while !rdr.eof() {
            match rdr.read_char() {
              '%' => {
                let bytes = rdr.read_bytes(2u);
                let ch = uint::parse_buf(bytes, 16u).get() as char;

                if full_url {
                    // Only decode some characters:
                    match ch {
                      // gen-delims:
                      ':' | '/' | '?' | '#' | '[' | ']' | '@' |

                      // sub-delims:
                      '!' | '$' | '&' | '"' | '(' | ')' | '*' |
                      '+' | ',' | ';' | '=' => {
                        str::push_char(out, '%');
                        str::push_char(out, bytes[0u] as char);
                        str::push_char(out, bytes[1u] as char);
                      }

                      ch => str::push_char(out, ch)
                    }
                } else {
                      str::push_char(out, ch);
                }
              }
              ch => str::push_char(out, ch)
            }
        }

        out
    }
}

/**
 * Decode a string encoded with percent encoding.
 *
 * This will only decode escape sequences generated by encode_uri.
 */
fn decode(s: ~str) -> ~str {
    decode_inner(s, true)
}

/**
 * Decode a string encoded with percent encoding.
 */
fn decode_component(s: ~str) -> ~str {
    decode_inner(s, false)
}

fn encode_plus(s: ~str) -> ~str {
    do io::with_str_reader(s) |rdr| {
        let mut out = ~"";

        while !rdr.eof() {
            let ch = rdr.read_byte() as char;
            match ch {
              'A' to 'Z' | 'a' to 'z' | '0' to '9' | '_' | '.' | '-' => {
                str::push_char(out, ch);
              }
              ' ' => str::push_char(out, '+'),
              _ => out += #fmt("%%%X", ch as uint)
            }
        }

        out
    }
}

/**
 * Encode a hashmap to the 'application/x-www-form-urlencoded' media type.
 */
fn encode_form_urlencoded(m: hashmap<~str, @DVec<@~str>>) -> ~str {
    let mut out = ~"";
    let mut first = true;

    for m.each |key, values| {
        let key = encode_plus(key);

        for (*values).each |value| {
            if first {
                first = false;
            } else {
                str::push_char(out, '&');
                first = false;
            }

            out += #fmt("%s=%s", key, encode_plus(*value));
        }
    }

    out
}

/**
 * Decode a string encoded with the 'application/x-www-form-urlencoded' media
 * type into a hashmap.
 */
fn decode_form_urlencoded(s: ~[u8]) ->
    map::hashmap<~str, @dvec::DVec<@~str>> {
    do io::with_bytes_reader(s) |rdr| {
        let m = str_hash();
        let mut key = ~"";
        let mut value = ~"";
        let mut parsing_key = true;

        while !rdr.eof() {
            match rdr.read_char() {
              '&' | ';' => {
                if key != ~"" && value != ~"" {
                    let values = match m.find(key) {
                      some(values) => values,
                      none => {
                        let values = @dvec();
                        m.insert(key, values);
                        values
                      }
                    };
                    (*values).push(@value)
                }

                parsing_key = true;
                key = ~"";
                value = ~"";
              }
              '=' => parsing_key = false,
              ch => {
                let ch = match ch {
                  '%' => {
                    uint::parse_buf(rdr.read_bytes(2u), 16u).get() as char
                  }
                  '+' => ' ',
                  ch => ch
                };

                if parsing_key {
                    str::push_char(key, ch)
                } else {
                    str::push_char(value, ch)
                }
              }
            }
        }

        if key != ~"" && value != ~"" {
            let values = match m.find(key) {
              some(values) => values,
              none => {
                let values = @dvec();
                m.insert(key, values);
                values
              }
            };
            (*values).push(@value)
        }

        m
    }
}


fn split_char_first(s: ~str, c: char) -> (~str, ~str) {
    let len = str::len(s);
    let mut index = len;
    let mut mat = 0;
    do io::with_str_reader(s) |rdr| {
        let mut ch : char;
        while !rdr.eof() {
            ch = rdr.read_byte() as char;
            if ch == c {
                // found a match, adjust markers
                index = rdr.tell()-1;
                mat = 1;
                break;
            }
        }
    }
    if index+mat == len {
        return (str::slice(s, 0, index), ~"");
    } else {
        return (str::slice(s, 0, index),
             str::slice(s, index + mat, str::len(s)));
    }
}

fn userinfo_from_str(uinfo: ~str) -> userinfo {
    let (user, p) = split_char_first(uinfo, ':');
    let pass = if str::len(p) == 0 {
        option::none
    } else {
        option::some(p)
    };
    return userinfo(user, pass);
}

fn userinfo_to_str(-userinfo: userinfo) -> ~str {
    if option::is_some(userinfo.pass) {
        return str::concat(~[copy userinfo.user, ~":",
                          option::unwrap(copy userinfo.pass),
                          ~"@"]);
    } else {
        return str::concat(~[copy userinfo.user, ~"@"]);
    }
}

fn query_from_str(rawquery: ~str) -> query {
    let mut query: query = ~[];
    if str::len(rawquery) != 0 {
        for str::split_char(rawquery, '&').each |p| {
            let (k, v) = split_char_first(p, '=');
            vec::push(query, (decode_component(k), decode_component(v)));
        };
    }
    return query;
}

fn query_to_str(query: query) -> ~str {
    let mut strvec = ~[];
    for query.each |kv| {
        let (k, v) = copy kv;
        strvec += ~[#fmt("%s=%s", encode_component(k), encode_component(v))];
    };
    return str::connect(strvec, ~"&");
}

// returns the scheme and the rest of the url, or a parsing error
fn get_scheme(rawurl: ~str) -> result::result<(~str, ~str), @~str> {
    for str::each_chari(rawurl) |i,c| {
        match c {
          'A' to 'Z' | 'a' to 'z' => again,
          '0' to '9' | '+' | '-' | '.' => {
            if i == 0 {
                return result::err(@~"url: Scheme must begin with a letter.");
            }
            again;
          }
          ':' => {
            if i == 0 {
                return result::err(@~"url: Scheme cannot be empty.");
            } else {
                return result::ok((rawurl.slice(0,i),
                                rawurl.slice(i+1,str::len(rawurl))));
            }
          }
          _ => {
            return result::err(@~"url: Invalid character in scheme.");
          }
        }
    };
    return result::err(@~"url: Scheme must be terminated with a colon.");
}

// returns userinfo, host, port, and unparsed part, or an error
fn get_authority(rawurl: ~str) ->
    result::result<(option<userinfo>, ~str, option<~str>, ~str), @~str> {
    if !str::starts_with(rawurl, ~"//") {
        // there is no authority.
        return result::ok((option::none, ~"", option::none, copy rawurl));
    }

    enum state {
        start, // starting state
        pass_host_port, // could be in user or port
        ip6_port, // either in ipv6 host or port
        ip6_host, // are in an ipv6 host
        in_host, // are in a host - may be ipv6, but don't know yet
        in_port // are in port
    }
    enum input {
        digit, // all digits
        hex, // digits and letters a-f
        unreserved // all other legal characters
    }
    let len = str::len(rawurl);
    let mut st : state = start;
    let mut in : input = digit; // most restricted, start here.

    let mut userinfo : option<userinfo> = option::none;
    let mut host : ~str = ~"";
    let mut port : option::option<~str> = option::none;

    let mut colon_count = 0;
    let mut pos : uint = 0, begin : uint = 2, end : uint = len;

    for str::each_chari(rawurl) |i,c| {
        if i < 2 { again; } // ignore the leading //

        // deal with input class first
        match c {
          '0' to '9' => (),
          'A' to 'F' | 'a' to 'f' => {
            if in == digit {
                in = hex;
            }
          }
          'G' to 'Z' | 'g' to 'z' | '-' | '.' | '_' | '~' | '%' |
          '&' |'\'' | '(' | ')' | '+' | '!' | '*' | ',' | ';' | '=' => {
            in = unreserved;
          }
          ':' | '@' | '?' | '#' | '/' => {
            // separators, don't change anything
          }
          _ => {
            return result::err(@~"Illegal character in authority");
          }
        }

        // now process states
        match c {
          ':' => {
            colon_count += 1;
            match st {
              start => {
                pos = i;
                st = pass_host_port;
              }
              pass_host_port => {
                // multiple colons means ipv6 address.
                if in == unreserved {
                    return result::err(
                        @~"Illegal characters in IPv6 address.");
                }
                st = ip6_host;
              }
              in_host => {
                pos = i;
                // can't be sure whether this is an ipv6 address or a port
                if in == unreserved {
                    return result::err(@~"Illegal characters in authority.");
                }
                st = ip6_port;
              }
              ip6_port => {
                if in == unreserved {
                    return result::err(@~"Illegal characters in authority.");
                }
                st = ip6_host;
              }
              ip6_host => {
                if colon_count > 7 {
                    host = str::slice(rawurl, begin, i);
                    pos = i;
                    st = in_port;
                }
              }
              _ => {
                return result::err(@~"Invalid ':' in authority.");
              }
            }
            in = digit; // reset input class
          }

          '@' => {
            in = digit; // reset input class
            colon_count = 0; // reset count
            match st {
              start => {
                let user = str::slice(rawurl, begin, i);
                userinfo = option::some({user : user,
                                         pass: option::none});
                st = in_host;
              }
              pass_host_port => {
                let user = str::slice(rawurl, begin, pos);
                let pass = str::slice(rawurl, pos+1, i);
                userinfo = option::some({user: user,
                                         pass: option::some(pass)});
                st = in_host;
              }
              _ => {
                return result::err(@~"Invalid '@' in authority.");
              }
            }
            begin = i+1;
          }

          '?' | '#' | '/' => {
            end = i;
            break;
          }
          _ => ()
        }
        end = i;
    }

    let end = end; // make end immutable so it can be captured

    let host_is_end_plus_one = || {
        end+1 == len
            && !['?', '#', '/'].contains(rawurl[end] as char)
    };

    // finish up
    match st {
      start => {
        if host_is_end_plus_one() {
            host = str::slice(rawurl, begin, end+1);
        } else {
            host = str::slice(rawurl, begin, end);
        }
      }
      pass_host_port | ip6_port => {
        if in != digit {
            return result::err(@~"Non-digit characters in port.");
        }
        host = str::slice(rawurl, begin, pos);
        port = option::some(str::slice(rawurl, pos+1, end));
      }
      ip6_host | in_host => {
        host = str::slice(rawurl, begin, end);
      }
      in_port => {
        if in != digit {
            return result::err(@~"Non-digit characters in port.");
        }
        port = option::some(str::slice(rawurl, pos+1, end));
      }
    }

    let rest = if host_is_end_plus_one() { ~"" }
    else { str::slice(rawurl, end, len) };
    return result::ok((userinfo, host, port, rest));
}


// returns the path and unparsed part of url, or an error
fn get_path(rawurl: ~str, authority : bool) ->
    result::result<(~str, ~str), @~str> {
    let len = str::len(rawurl);
    let mut end = len;
    for str::each_chari(rawurl) |i,c| {
        match c {
          'A' to 'Z' | 'a' to 'z' | '0' to '9' | '&' |'\'' | '(' | ')' | '.'
          | '@' | ':' | '%' | '/' | '+' | '!' | '*' | ',' | ';' | '='
          | '_' | '-' => {
            again;
          }
          '?' | '#' => {
            end = i;
            break;
          }
          _ => return result::err(@~"Invalid character in path.")
        }
    }

    if authority {
        if end != 0 && !str::starts_with(rawurl, ~"/") {
            return result::err(@~"Non-empty path must begin with\
                               '/' in presence of authority.");
        }
    }

    return result::ok((decode_component(str::slice(rawurl, 0, end)),
                    str::slice(rawurl, end, len)));
}

// returns the parsed query and the fragment, if present
fn get_query_fragment(rawurl: ~str) ->
    result::result<(query, option<~str>), @~str> {
    if !str::starts_with(rawurl, ~"?") {
        if str::starts_with(rawurl, ~"#") {
            let f = decode_component(str::slice(rawurl,
                                                1,
                                                str::len(rawurl)));
            return result::ok((~[], option::some(f)));
        } else {
            return result::ok((~[], option::none));
        }
    }
    let (q, r) = split_char_first(str::slice(rawurl, 1,
                                             str::len(rawurl)), '#');
    let f = if str::len(r) != 0 {
        option::some(decode_component(r)) } else { option::none };
    return result::ok((query_from_str(q), f));
}

/**
 * Parse a `str` to a `url`
 *
 * # Arguments
 *
 * `rawurl` - a string representing a full url, including scheme.
 *
 * # Returns
 *
 * a `url` that contains the parsed representation of the url.
 *
 */

fn from_str(rawurl: ~str) -> result::result<url, ~str> {
    // scheme
    let mut schm = get_scheme(rawurl);
    if result::is_err(schm) {
        return result::err(copy *result::get_err(schm));
    }
    let (scheme, rest) = result::unwrap(schm);

    // authority
    let mut auth = get_authority(rest);
    if result::is_err(auth) {
        return result::err(copy *result::get_err(auth));
    }
    let (userinfo, host, port, rest) = result::unwrap(auth);

    // path
    let has_authority = if host == ~"" { false } else { true };
    let mut pth = get_path(rest, has_authority);
    if result::is_err(pth) {
        return result::err(copy *result::get_err(pth));
    }
    let (path, rest) = result::unwrap(pth);

    // query and fragment
    let mut qry = get_query_fragment(rest);
    if result::is_err(qry) {
        return result::err(copy *result::get_err(qry));
    }
    let (query, fragment) = result::unwrap(qry);

    return result::ok(url(scheme, userinfo, host,
                       port, path, query, fragment));
}

/**
 * Format a `url` as a string
 *
 * # Arguments
 *
 * `url` - a url.
 *
 * # Returns
 *
 * a `str` that contains the formatted url. Note that this will usually
 * be an inverse of `from_str` but might strip out unneeded separators.
 * for example, "http://somehost.com?", when parsed and formatted, will
 * result in just "http://somehost.com".
 *
 */
fn to_str(url: url) -> ~str {
    let user = if option::is_some(url.user) {
      userinfo_to_str(option::unwrap(copy url.user))
    } else {
       ~""
    };
    let authority = if str::len(url.host) != 0 {
        str::concat(~[~"//", user, copy url.host])
    } else {
        ~""
    };
    let query = if url.query.len() == 0 {
        ~""
    } else {
        str::concat(~[~"?", query_to_str(url.query)])
    };
    let fragment = if option::is_some(url.fragment) {
        str::concat(~[~"#", encode_component(
            option::unwrap(copy url.fragment))])
    } else {
        ~""
    };

    return str::concat(~[copy url.scheme,
                      ~":",
                      authority,
                      copy url.path,
                      query,
                      fragment]);
}

impl url: to_str::ToStr {
    fn to_str() -> ~str {
        to_str(self)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_split_char_first() {
        let (u,v) = split_char_first(~"hello, sweet world", ',');
        assert u == ~"hello";
        assert v == ~" sweet world";

        let (u,v) = split_char_first(~"hello sweet world", ',');
        assert u == ~"hello sweet world";
        assert v == ~"";
    }

    #[test]
    fn test_get_authority() {
        let (u, h, p, r) = result::unwrap(get_authority(
            ~"//user:pass@rust-lang.org/something"));
        assert u == option::some({user: ~"user",
                                  pass: option::some(~"pass")});
        assert h == ~"rust-lang.org";
        assert option::is_none(p);
        assert r == ~"/something";

        let (u, h, p, r) = result::unwrap(get_authority(
            ~"//rust-lang.org:8000?something"));
        assert option::is_none(u);
        assert h == ~"rust-lang.org";
        assert p == option::some(~"8000");
        assert r == ~"?something";

        let (u, h, p, r) = result::unwrap(get_authority(
            ~"//rust-lang.org#blah"));
        assert option::is_none(u);
        assert h == ~"rust-lang.org";
        assert option::is_none(p);
        assert r == ~"#blah";

        // ipv6 tests
        let (_, h, _, _) = result::unwrap(get_authority(
            ~"//2001:0db8:85a3:0042:0000:8a2e:0370:7334#blah"));
        assert h == ~"2001:0db8:85a3:0042:0000:8a2e:0370:7334";

        let (_, h, p, _) = result::unwrap(get_authority(
            ~"//2001:0db8:85a3:0042:0000:8a2e:0370:7334:8000#blah"));
        assert h == ~"2001:0db8:85a3:0042:0000:8a2e:0370:7334";
        assert p == option::some(~"8000");

        let (u, h, p, _) = result::unwrap(get_authority(
            ~"//us:p@2001:0db8:85a3:0042:0000:8a2e:0370:7334:8000#blah"));
        assert u == option::some({user: ~"us", pass : option::some(~"p")});
        assert h == ~"2001:0db8:85a3:0042:0000:8a2e:0370:7334";
        assert p == option::some(~"8000");

        // invalid authorities;
        assert result::is_err(get_authority(
            ~"//user:pass@rust-lang:something"));
        assert result::is_err(get_authority(
            ~"//user@rust-lang:something:/path"));
        assert result::is_err(get_authority(
            ~"//2001:0db8:85a3:0042:0000:8a2e:0370:7334:800a"));
        assert result::is_err(get_authority(
            ~"//2001:0db8:85a3:0042:0000:8a2e:0370:7334:8000:00"));

        // these parse as empty, because they don't start with '//'
        let (_, h, _, _) = result::unwrap(
            get_authority(~"user:pass@rust-lang"));
        assert h == ~"";
        let (_, h, _, _) = result::unwrap(
            get_authority(~"rust-lang.org"));
        assert h == ~"";

    }

    #[test]
    fn test_get_path() {
        let (p, r) = result::unwrap(get_path(
            ~"/something+%20orother", true));
        assert p == ~"/something+ orother";
        assert r == ~"";
        let (p, r) = result::unwrap(get_path(
            ~"test@email.com#fragment", false));
        assert p == ~"test@email.com";
        assert r == ~"#fragment";
        let (p, r) = result::unwrap(get_path(~"/gen/:addr=?q=v", false));
        assert p == ~"/gen/:addr=";
        assert r == ~"?q=v";

        //failure cases
        assert result::is_err(get_path(~"something?q", true));

    }

    #[test]
    fn test_url_parse() {
        let url = ~"http://user:pass@rust-lang.org/doc?s=v#something";

        let up = from_str(url);
        let u = result::unwrap(up);
        assert u.scheme == ~"http";
        assert option::unwrap(copy u.user).user == ~"user";
        assert option::unwrap(copy option::unwrap(copy u.user).pass)
            == ~"pass";
        assert u.host == ~"rust-lang.org";
        assert u.path == ~"/doc";
        assert u.query.find(|kv| kv.first() == ~"s").get().second() == ~"v";
        assert option::unwrap(copy u.fragment) == ~"something";
    }

    #[test]
    fn test_url_parse_host_slash() {
        let urlstr = ~"http://0.42.42.42/";
        let url = from_str(urlstr).get();
        #debug("url: %?", url);
        assert url.host == ~"0.42.42.42";
        assert url.path == ~"/";
    }

    #[test]
    fn test_url_with_underscores() {
        let urlstr = ~"http://dotcom.com/file_name.html";
        let url = from_str(urlstr).get();
        #debug("url: %?", url);
        assert url.path == ~"/file_name.html";
    }

    #[test]
    fn test_url_with_dashes() {
        let urlstr = ~"http://dotcom.com/file-name.html";
        let url = from_str(urlstr).get();
        #debug("url: %?", url);
        assert url.path == ~"/file-name.html";
    }

    #[test]
    fn test_no_scheme() {
        assert result::is_err(get_scheme(~"noschemehere.html"));
    }

    #[test]
    fn test_invalid_scheme_errors() {
        assert result::is_err(from_str(~"99://something"));
        assert result::is_err(from_str(~"://something"));
    }

    #[test]
    fn test_full_url_parse_and_format() {
        let url = ~"http://user:pass@rust-lang.org/doc?s=v#something";
        assert to_str(result::unwrap(from_str(url))) == url;
    }

    #[test]
    fn test_userless_url_parse_and_format() {
        let url = ~"http://rust-lang.org/doc?s=v#something";
        assert to_str(result::unwrap(from_str(url))) == url;
    }

    #[test]
    fn test_queryless_url_parse_and_format() {
        let url = ~"http://user:pass@rust-lang.org/doc#something";
        assert to_str(result::unwrap(from_str(url))) == url;
    }

    #[test]
    fn test_empty_query_url_parse_and_format() {
        let url = ~"http://user:pass@rust-lang.org/doc?#something";
        let should_be = ~"http://user:pass@rust-lang.org/doc#something";
        assert to_str(result::unwrap(from_str(url))) == should_be;
    }

    #[test]
    fn test_fragmentless_url_parse_and_format() {
        let url = ~"http://user:pass@rust-lang.org/doc?q=v";
        assert to_str(result::unwrap(from_str(url))) == url;
    }

    #[test]
    fn test_minimal_url_parse_and_format() {
        let url = ~"http://rust-lang.org/doc";
        assert to_str(result::unwrap(from_str(url))) == url;
    }

    #[test]
    fn test_scheme_host_only_url_parse_and_format() {
        let url = ~"http://rust-lang.org";
        assert to_str(result::unwrap(from_str(url))) == url;
    }

    #[test]
    fn test_pathless_url_parse_and_format() {
        let url = ~"http://user:pass@rust-lang.org?q=v#something";
        assert to_str(result::unwrap(from_str(url))) == url;
    }

    #[test]
    fn test_scheme_host_fragment_only_url_parse_and_format() {
        let url = ~"http://rust-lang.org#something";
        assert to_str(result::unwrap(from_str(url))) == url;
    }

    #[test]
    fn test_url_component_encoding() {
        let url = ~"http://rust-lang.org/doc%20uments?ba%25d%20=%23%26%2B";
        let u = result::unwrap(from_str(url));
        assert u.path == ~"/doc uments";
        assert u.query.find(|kv| kv.first() == ~"ba%d ")
            .get().second() == ~"#&+";
    }

    #[test]
    fn test_url_without_authority() {
        let url = ~"mailto:test@email.com";
        assert to_str(result::unwrap(from_str(url))) == url;
    }

    #[test]
    fn test_encode() {
        assert encode(~"") == ~"";
        assert encode(~"http://example.com") == ~"http://example.com";
        assert encode(~"foo bar% baz") == ~"foo%20bar%25%20baz";
        assert encode(~" ") == ~"%20";
        assert encode(~"!") == ~"!";
        assert encode(~"\"") == ~"\"";
        assert encode(~"#") == ~"#";
        assert encode(~"$") == ~"$";
        assert encode(~"%") == ~"%25";
        assert encode(~"&") == ~"&";
        assert encode(~"'") == ~"%27";
        assert encode(~"(") == ~"(";
        assert encode(~")") == ~")";
        assert encode(~"*") == ~"*";
        assert encode(~"+") == ~"+";
        assert encode(~",") == ~",";
        assert encode(~"/") == ~"/";
        assert encode(~":") == ~":";
        assert encode(~";") == ~";";
        assert encode(~"=") == ~"=";
        assert encode(~"?") == ~"?";
        assert encode(~"@") == ~"@";
        assert encode(~"[") == ~"[";
        assert encode(~"]") == ~"]";
    }

    #[test]
    fn test_encode_component() {
        assert encode_component(~"") == ~"";
        assert encode_component(~"http://example.com") ==
            ~"http%3A%2F%2Fexample.com";
        assert encode_component(~"foo bar% baz") == ~"foo%20bar%25%20baz";
        assert encode_component(~" ") == ~"%20";
        assert encode_component(~"!") == ~"%21";
        assert encode_component(~"#") == ~"%23";
        assert encode_component(~"$") == ~"%24";
        assert encode_component(~"%") == ~"%25";
        assert encode_component(~"&") == ~"%26";
        assert encode_component(~"'") == ~"%27";
        assert encode_component(~"(") == ~"%28";
        assert encode_component(~")") == ~"%29";
        assert encode_component(~"*") == ~"%2A";
        assert encode_component(~"+") == ~"%2B";
        assert encode_component(~",") == ~"%2C";
        assert encode_component(~"/") == ~"%2F";
        assert encode_component(~":") == ~"%3A";
        assert encode_component(~";") == ~"%3B";
        assert encode_component(~"=") == ~"%3D";
        assert encode_component(~"?") == ~"%3F";
        assert encode_component(~"@") == ~"%40";
        assert encode_component(~"[") == ~"%5B";
        assert encode_component(~"]") == ~"%5D";
    }

    #[test]
    fn test_decode() {
        assert decode(~"") == ~"";
        assert decode(~"abc/def 123") == ~"abc/def 123";
        assert decode(~"abc%2Fdef%20123") == ~"abc%2Fdef 123";
        assert decode(~"%20") == ~" ";
        assert decode(~"%21") == ~"%21";
        assert decode(~"%22") == ~"%22";
        assert decode(~"%23") == ~"%23";
        assert decode(~"%24") == ~"%24";
        assert decode(~"%25") == ~"%";
        assert decode(~"%26") == ~"%26";
        assert decode(~"%27") == ~"'";
        assert decode(~"%28") == ~"%28";
        assert decode(~"%29") == ~"%29";
        assert decode(~"%2A") == ~"%2A";
        assert decode(~"%2B") == ~"%2B";
        assert decode(~"%2C") == ~"%2C";
        assert decode(~"%2F") == ~"%2F";
        assert decode(~"%3A") == ~"%3A";
        assert decode(~"%3B") == ~"%3B";
        assert decode(~"%3D") == ~"%3D";
        assert decode(~"%3F") == ~"%3F";
        assert decode(~"%40") == ~"%40";
        assert decode(~"%5B") == ~"%5B";
        assert decode(~"%5D") == ~"%5D";
    }

    #[test]
    fn test_decode_component() {
        assert decode_component(~"") == ~"";
        assert decode_component(~"abc/def 123") == ~"abc/def 123";
        assert decode_component(~"abc%2Fdef%20123") == ~"abc/def 123";
        assert decode_component(~"%20") == ~" ";
        assert decode_component(~"%21") == ~"!";
        assert decode_component(~"%22") == ~"\"";
        assert decode_component(~"%23") == ~"#";
        assert decode_component(~"%24") == ~"$";
        assert decode_component(~"%25") == ~"%";
        assert decode_component(~"%26") == ~"&";
        assert decode_component(~"%27") == ~"'";
        assert decode_component(~"%28") == ~"(";
        assert decode_component(~"%29") == ~")";
        assert decode_component(~"%2A") == ~"*";
        assert decode_component(~"%2B") == ~"+";
        assert decode_component(~"%2C") == ~",";
        assert decode_component(~"%2F") == ~"/";
        assert decode_component(~"%3A") == ~":";
        assert decode_component(~"%3B") == ~";";
        assert decode_component(~"%3D") == ~"=";
        assert decode_component(~"%3F") == ~"?";
        assert decode_component(~"%40") == ~"@";
        assert decode_component(~"%5B") == ~"[";
        assert decode_component(~"%5D") == ~"]";
    }

    #[test]
    fn test_encode_form_urlencoded() {
        let m = str_hash();
        assert encode_form_urlencoded(m) == ~"";

        m.insert(~"", @dvec());
        m.insert(~"foo", @dvec());
        assert encode_form_urlencoded(m) == ~"";

        let m = str_hash();
        m.insert(~"foo", @dvec::from_vec(~[mut @~"bar", @~"123"]));
        assert encode_form_urlencoded(m) == ~"foo=bar&foo=123";

        let m = str_hash();
        m.insert(~"foo bar", @dvec::from_vec(~[mut @~"abc", @~"12 = 34"]));
        assert encode_form_urlencoded(m) == ~"foo+bar=abc&foo+bar=12+%3D+34";
    }

    #[test]
    fn test_decode_form_urlencoded() {
        import map::hash_from_strs;

        assert decode_form_urlencoded(~[]) == str_hash();

        let s = str::bytes(~"a=1&foo+bar=abc&foo+bar=12+%3D+34");
        assert decode_form_urlencoded(s) == hash_from_strs(~[
            (~"a", @dvec::from_elem(@~"1")),
            (~"foo bar", @dvec::from_vec(~[mut @~"abc", @~"12 = 34"]))
        ]);
    }

}

