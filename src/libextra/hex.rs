// Copyright 2013 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Hex binary-to-text encoding
use std::str;
use std::vec;

/// A trait for converting a value to hexadecimal encoding
pub trait ToHex {
    /// Converts the value of `self` to a hex value, returning the owned
    /// string.
    fn to_hex(&self) -> ~str;
}

static CHARS: [char, ..16] = ['0', '1', '2', '3', '4', '5', '6', '7', '8', '9',
                              'a', 'b', 'c', 'd', 'e', 'f'];

impl<'self> ToHex for &'self [u8] {
    /**
     * Turn a vector of `u8` bytes into a hexadecimal string.
     *
     * # Example
     *
     * ~~~ {.rust}
     * extern mod extra;
     * use extra::hex::ToHex;
     *
     * fn main () {
     *     let str = [52,32].to_hex();
     *     printfln!("%s", str);
     * }
     * ~~~
     */
    fn to_hex(&self) -> ~str {
        let mut s = str::with_capacity(self.len() * 2);
        for &byte in self.iter() {
            s.push_char(CHARS[byte >> 4]);
            s.push_char(CHARS[byte & 0xf]);
        }

        s
    }
}

impl<'self> ToHex for &'self str {
    /**
     * Convert any string (literal, `@`, `&`, or `~`) to hexadecimal encoding.
     *
     *
     * # Example
     *
     * ~~~ {.rust}
         * extern mod extra;
     * use extra::ToHex;
     *
     * fn main () {
     *     let str = "Hello, World".to_hex();
     *     printfln!("%s", str);
     * }
     * ~~~
     *
     */
    fn to_hex(&self) -> ~str {
        self.as_bytes().to_hex()
    }
}

/// A trait for converting hexadecimal encoded values
pub trait FromHex {
    /// Converts the value of `self`, interpreted as base64 encoded data, into
    /// an owned vector of bytes, returning the vector.
    fn from_hex(&self) -> Result<~[u8], ~str>;
}

impl<'self> FromHex for &'self [u8] {
    /**
     * Convert hexadecimal `u8` vector into u8 byte values.
     * Every 2 encoded characters is converted into 1 octet.
     *
     * # Example
     *
     * ~~~ {.rust}
     * extern mod extra;
     * use extra::hex::{ToHex, FromHex};
     *
     * fn main () {
     *     let str = [52,32].to_hex();
     *     printfln!("%s", str);
     *     let bytes = str.from_hex().get();
     *     printfln!("%?", bytes);
     * }
     * ~~~
     */
    fn from_hex(&self) -> Result<~[u8], ~str> {
        // This may be an overestimate if there is any whitespace
        let mut b = vec::with_capacity(self.len() / 2);
        let mut modulus = 0;
        let mut buf = 0u8;

        for &byte in self.iter() {
            buf <<= 4;

            match byte as char {
                'A'..'F' => buf |= byte - ('A' as u8) + 10,
                'a'..'f' => buf |= byte - ('a' as u8) + 10,
                '0'..'9' => buf |= byte - ('0' as u8),
                ' '|'\r'|'\n' => {
                    buf >>= 4;
                    loop
                }
                _ => return Err(~"Invalid hex char")
            }

            modulus += 1;
            if modulus == 2 {
                modulus = 0;
                b.push(buf);
            }
        }

        match modulus {
            0 => Ok(b),
            _ => Err(~"Invalid input length")
        }
    }
}

impl<'self> FromHex for &'self str {
    /**
     * Convert any hexadecimal encoded string (literal, `@`, `&`, or `~`)
     * to the byte values it encodes.
     *
     * You can use the `from_bytes` function in `std::str`
     * to turn a `[u8]` into a string with characters corresponding to those
     * values.
     *
     * # Example
     *
     * This converts a string literal to hexadecimal and back.
     *
     * ~~~ {.rust}
     * extern mod extra;
     * use extra::hex::{FromHex, ToHex};
     * use std::str;
     *
     * fn main () {
     *     let hello_str = "Hello, World".to_hex();
     *     printfln!("%s", hello_str);
     *     let bytes = hello_str.from_hex().get();
     *     printfln!("%?", bytes);
     *     let result_str = str::from_bytes(bytes);
     *     printfln!("%s", result_str);
     * }
     * ~~~
     */
    fn from_hex(&self) -> Result<~[u8], ~str> {
        self.as_bytes().from_hex()
    }
}

#[cfg(test)]
mod tests {
    use test::BenchHarness;
    use hex::*;

    #[test]
    pub fn test_to_hex() {
        assert_eq!("foobar".to_hex(), ~"666f6f626172");
    }

    #[test]
    pub fn test_from_hex_okay() {
        assert_eq!("666f6f626172".from_hex().get(),
                   "foobar".as_bytes().to_owned());
        assert_eq!("666F6F626172".from_hex().get(),
                   "foobar".as_bytes().to_owned());
    }

    #[test]
    pub fn test_from_hex_odd_len() {
        assert!("666".from_hex().is_err());
        assert!("66 6".from_hex().is_err());
    }

    #[test]
    pub fn test_from_hex_invalid_char() {
        assert!("66y6".from_hex().is_err());
    }

    #[test]
    pub fn test_from_hex_ignores_whitespace() {
        assert_eq!("666f 6f6\r\n26172 ".from_hex().get(),
                   "foobar".as_bytes().to_owned());
    }

    #[test]
    pub fn test_to_hex_all_bytes() {
        for i in range(0, 256) {
            assert_eq!([i as u8].to_hex(), fmt!("%02x", i as uint));
        }
    }

    #[test]
    pub fn test_from_hex_all_bytes() {
        for i in range(0, 256) {
            assert_eq!(fmt!("%02x", i as uint).from_hex().get(), ~[i as u8]);
            assert_eq!(fmt!("%02X", i as uint).from_hex().get(), ~[i as u8]);
        }
    }

    #[bench]
    pub fn bench_to_hex(bh: & mut BenchHarness) {
        let s = "イロハニホヘト チリヌルヲ ワカヨタレソ ツネナラム \
                 ウヰノオクヤマ ケフコエテ アサキユメミシ ヱヒモセスン";
        do bh.iter {
            s.to_hex();
        }
        bh.bytes = s.len() as u64;
    }

    #[bench]
    pub fn bench_from_hex(bh: & mut BenchHarness) {
        let s = "イロハニホヘト チリヌルヲ ワカヨタレソ ツネナラム \
                 ウヰノオクヤマ ケフコエテ アサキユメミシ ヱヒモセスン";
        let b = s.to_hex();
        do bh.iter {
            b.from_hex();
        }
        bh.bytes = b.len() as u64;
    }
}
