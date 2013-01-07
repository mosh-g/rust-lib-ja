// Copyright 2012 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

// The crate store - a central repo for information collected about external
// crates and libraries

use metadata::creader;
use metadata::cstore;
use metadata::decoder;

use core::option;
use core::str;
use core::vec;
use std::map::HashMap;
use std::map;
use std;
use syntax::{ast, attr};
use syntax::parse::token::ident_interner;

export CStore;
export cnum_map;
export crate_metadata;
export mk_cstore;
export get_crate_data;
export set_crate_data;
export get_crate_hash;
export get_crate_vers;
export have_crate_data;
export iter_crate_data;
export add_used_crate_file;
export get_used_crate_files;
export add_used_library;
export get_used_libraries;
export add_used_link_args;
export get_used_link_args;
export add_use_stmt_cnum;
export find_use_stmt_cnum;
export get_dep_hashes;


// A map from external crate numbers (as decoded from some crate file) to
// local crate numbers (as generated during this session). Each external
// crate may refer to types in other external crates, and each has their
// own crate numbers.
type cnum_map = map::HashMap<ast::crate_num, ast::crate_num>;

type crate_metadata = @{name: ~str,
                        data: @~[u8],
                        cnum_map: cnum_map,
                        cnum: ast::crate_num};

// This is a bit of an experiment at encapsulating the data in cstore. By
// keeping all the data in a non-exported enum variant, it's impossible for
// other modules to access the cstore's private data. This could also be
// achieved with an obj, but at the expense of a vtable. Not sure if this is a
// good pattern or not.
enum CStore { private(cstore_private), }

type cstore_private =
    @{metas: map::HashMap<ast::crate_num, crate_metadata>,
      use_crate_map: use_crate_map,
      mut used_crate_files: ~[Path],
      mut used_libraries: ~[~str],
      mut used_link_args: ~[~str],
      intr: @ident_interner};

// Map from node_id's of local use statements to crate numbers
type use_crate_map = map::HashMap<ast::node_id, ast::crate_num>;

// Internal method to retrieve the data from the cstore
pure fn p(cstore: CStore) -> cstore_private {
    match cstore { private(p) => p }
}

fn mk_cstore(intr: @ident_interner) -> CStore {
    let meta_cache = map::HashMap();
    let crate_map = map::HashMap();
    return private(@{metas: meta_cache,
                     use_crate_map: crate_map,
                     mut used_crate_files: ~[],
                     mut used_libraries: ~[],
                     mut used_link_args: ~[],
                     intr: intr});
}

fn get_crate_data(cstore: CStore, cnum: ast::crate_num) -> crate_metadata {
    return p(cstore).metas.get(cnum);
}

fn get_crate_hash(cstore: CStore, cnum: ast::crate_num) -> ~str {
    let cdata = get_crate_data(cstore, cnum);
    return decoder::get_crate_hash(cdata.data);
}

fn get_crate_vers(cstore: CStore, cnum: ast::crate_num) -> ~str {
    let cdata = get_crate_data(cstore, cnum);
    return decoder::get_crate_vers(cdata.data);
}

fn set_crate_data(cstore: CStore,
                  cnum: ast::crate_num,
                  data: crate_metadata) {
    p(cstore).metas.insert(cnum, data);
}

fn have_crate_data(cstore: CStore, cnum: ast::crate_num) -> bool {
    return p(cstore).metas.contains_key(cnum);
}

fn iter_crate_data(cstore: CStore, i: fn(ast::crate_num, crate_metadata)) {
    for p(cstore).metas.each |k,v| { i(k, v);};
}

fn add_used_crate_file(cstore: CStore, lib: &Path) {
    if !vec::contains(p(cstore).used_crate_files, lib) {
        p(cstore).used_crate_files.push(copy *lib);
    }
}

fn get_used_crate_files(cstore: CStore) -> ~[Path] {
    return p(cstore).used_crate_files;
}

fn add_used_library(cstore: CStore, lib: ~str) -> bool {
    assert lib != ~"";

    if vec::contains(p(cstore).used_libraries, &lib) { return false; }
    p(cstore).used_libraries.push(lib);
    return true;
}

fn get_used_libraries(cstore: CStore) -> ~[~str] {
    return p(cstore).used_libraries;
}

fn add_used_link_args(cstore: CStore, args: ~str) {
    p(cstore).used_link_args.push_all(str::split_char(args, ' '));
}

fn get_used_link_args(cstore: CStore) -> ~[~str] {
    return p(cstore).used_link_args;
}

fn add_use_stmt_cnum(cstore: CStore, use_id: ast::node_id,
                     cnum: ast::crate_num) {
    p(cstore).use_crate_map.insert(use_id, cnum);
}

fn find_use_stmt_cnum(cstore: CStore,
                      use_id: ast::node_id) -> Option<ast::crate_num> {
    p(cstore).use_crate_map.find(use_id)
}

// returns hashes of crates directly used by this crate. Hashes are
// sorted by crate name.
fn get_dep_hashes(cstore: CStore) -> ~[~str] {
    type crate_hash = {name: ~str, hash: ~str};
    let mut result = ~[];

    for p(cstore).use_crate_map.each_value |cnum| {
        let cdata = cstore::get_crate_data(cstore, cnum);
        let hash = decoder::get_crate_hash(cdata.data);
        debug!("Add hash[%s]: %s", cdata.name, hash);
        result.push({name: cdata.name, hash: hash});
    };
    pure fn lteq(a: &crate_hash, b: &crate_hash) -> bool {a.name <= b.name}
    let sorted = std::sort::merge_sort(result, lteq);
    debug!("sorted:");
    for sorted.each |x| {
        debug!("  hash[%s]: %s", x.name, x.hash);
    }
    fn mapper(ch: &crate_hash) -> ~str { return ch.hash; }
    return vec::map(sorted, mapper);
}

// Local Variables:
// mode: rust
// fill-column: 78;
// indent-tabs-mode: nil
// c-basic-offset: 4
// buffer-file-coding-system: utf-8-unix
// End:
