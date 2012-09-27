use serialization2;

// Simple Extensible Binary Markup Language (ebml) reader and writer on a
// cursor model. See the specification here:
//     http://www.matroska.org/technical/specs/rfc/index.html
export Doc;
export doc_at;
export maybe_get_doc;
export get_doc;
export docs;
export tagged_docs;
export doc_data;
export doc_as_str;
export doc_as_u8;
export doc_as_u16;
export doc_as_u32;
export doc_as_u64;
export doc_as_i8;
export doc_as_i16;
export doc_as_i32;
export doc_as_i64;
export Serializer;
export Deserializer;
export with_doc_data;
export get_doc;
export extensions;

struct EbmlTag {
    id: uint,
    size: uint,
}

struct EbmlState {
    ebml_tag: EbmlTag,
    tag_pos: uint,
    data_pos: uint,
}

// FIXME (#2739): When we have module renaming, make "reader" and "writer"
// separate modules within this file.

// ebml reading
struct Doc {
    data: @~[u8],
    start: uint,
    end: uint,
}

struct TaggedDoc {
    tag: uint,
    doc: Doc,
}

impl Doc: ops::Index<uint,Doc> {
    pure fn index(+tag: uint) -> Doc {
        unsafe {
            get_doc(self, tag)
        }
    }
}

fn vuint_at(data: &[u8], start: uint) -> {val: uint, next: uint} {
    let a = data[start];
    if a & 0x80u8 != 0u8 {
        return {val: (a & 0x7fu8) as uint, next: start + 1u};
    }
    if a & 0x40u8 != 0u8 {
        return {val: ((a & 0x3fu8) as uint) << 8u |
                 (data[start + 1u] as uint),
             next: start + 2u};
    } else if a & 0x20u8 != 0u8 {
        return {val: ((a & 0x1fu8) as uint) << 16u |
                 (data[start + 1u] as uint) << 8u |
                 (data[start + 2u] as uint),
             next: start + 3u};
    } else if a & 0x10u8 != 0u8 {
        return {val: ((a & 0x0fu8) as uint) << 24u |
                 (data[start + 1u] as uint) << 16u |
                 (data[start + 2u] as uint) << 8u |
                 (data[start + 3u] as uint),
             next: start + 4u};
    } else { error!("vint too big"); fail; }
}

fn Doc(data: @~[u8]) -> Doc {
    Doc { data: data, start: 0u, end: vec::len::<u8>(*data) }
}

fn doc_at(data: @~[u8], start: uint) -> TaggedDoc {
    let elt_tag = vuint_at(*data, start);
    let elt_size = vuint_at(*data, elt_tag.next);
    let end = elt_size.next + elt_size.val;
    TaggedDoc {
        tag: elt_tag.val,
        doc: Doc { data: data, start: elt_size.next, end: end }
    }
}

fn maybe_get_doc(d: Doc, tg: uint) -> Option<Doc> {
    let mut pos = d.start;
    while pos < d.end {
        let elt_tag = vuint_at(*d.data, pos);
        let elt_size = vuint_at(*d.data, elt_tag.next);
        pos = elt_size.next + elt_size.val;
        if elt_tag.val == tg {
            return Some(Doc { data: d.data, start: elt_size.next, end: pos });
        }
    }
    None
}

fn get_doc(d: Doc, tg: uint) -> Doc {
    match maybe_get_doc(d, tg) {
      Some(d) => d,
      None => {
        error!("failed to find block with tag %u", tg);
        fail;
      }
    }
}

fn docs(d: Doc, it: fn(uint, Doc) -> bool) {
    let mut pos = d.start;
    while pos < d.end {
        let elt_tag = vuint_at(*d.data, pos);
        let elt_size = vuint_at(*d.data, elt_tag.next);
        pos = elt_size.next + elt_size.val;
        let doc = Doc { data: d.data, start: elt_size.next, end: pos };
        if !it(elt_tag.val, doc) {
            break;
        }
    }
}

fn tagged_docs(d: Doc, tg: uint, it: fn(Doc) -> bool) {
    let mut pos = d.start;
    while pos < d.end {
        let elt_tag = vuint_at(*d.data, pos);
        let elt_size = vuint_at(*d.data, elt_tag.next);
        pos = elt_size.next + elt_size.val;
        if elt_tag.val == tg {
            let doc = Doc { data: d.data, start: elt_size.next, end: pos };
            if !it(doc) {
                break;
            }
        }
    }
}

fn doc_data(d: Doc) -> ~[u8] { vec::slice::<u8>(*d.data, d.start, d.end) }

fn with_doc_data<T>(d: Doc, f: fn(x: &[u8]) -> T) -> T {
    f(vec::view(*d.data, d.start, d.end))
}

fn doc_as_str(d: Doc) -> ~str { str::from_bytes(doc_data(d)) }

fn doc_as_u8(d: Doc) -> u8 {
    assert d.end == d.start + 1u;
    (*d.data)[d.start]
}

fn doc_as_u16(d: Doc) -> u16 {
    assert d.end == d.start + 2u;
    io::u64_from_be_bytes(*d.data, d.start, 2u) as u16
}

fn doc_as_u32(d: Doc) -> u32 {
    assert d.end == d.start + 4u;
    io::u64_from_be_bytes(*d.data, d.start, 4u) as u32
}

fn doc_as_u64(d: Doc) -> u64 {
    assert d.end == d.start + 8u;
    io::u64_from_be_bytes(*d.data, d.start, 8u)
}

fn doc_as_i8(d: Doc) -> i8 { doc_as_u8(d) as i8 }
fn doc_as_i16(d: Doc) -> i16 { doc_as_u16(d) as i16 }
fn doc_as_i32(d: Doc) -> i32 { doc_as_u32(d) as i32 }
fn doc_as_i64(d: Doc) -> i64 { doc_as_u64(d) as i64 }

// ebml writing
struct Serializer {
    writer: io::Writer,
    priv mut size_positions: ~[uint],
}

fn write_sized_vuint(w: io::Writer, n: uint, size: uint) {
    match size {
      1u => w.write(&[0x80u8 | (n as u8)]),
      2u => w.write(&[0x40u8 | ((n >> 8_u) as u8), n as u8]),
      3u => w.write(&[0x20u8 | ((n >> 16_u) as u8), (n >> 8_u) as u8,
                      n as u8]),
      4u => w.write(&[0x10u8 | ((n >> 24_u) as u8), (n >> 16_u) as u8,
                      (n >> 8_u) as u8, n as u8]),
      _ => fail fmt!("vint to write too big: %?", n)
    };
}

fn write_vuint(w: io::Writer, n: uint) {
    if n < 0x7f_u { write_sized_vuint(w, n, 1u); return; }
    if n < 0x4000_u { write_sized_vuint(w, n, 2u); return; }
    if n < 0x200000_u { write_sized_vuint(w, n, 3u); return; }
    if n < 0x10000000_u { write_sized_vuint(w, n, 4u); return; }
    fail fmt!("vint to write too big: %?", n);
}

fn Serializer(w: io::Writer) -> Serializer {
    let size_positions: ~[uint] = ~[];
    Serializer { writer: w, mut size_positions: size_positions }
}

// FIXME (#2741): Provide a function to write the standard ebml header.
impl Serializer {
    fn start_tag(tag_id: uint) {
        debug!("Start tag %u", tag_id);

        // Write the enum ID:
        write_vuint(self.writer, tag_id);

        // Write a placeholder four-byte size.
        self.size_positions.push(self.writer.tell());
        let zeroes: &[u8] = &[0u8, 0u8, 0u8, 0u8];
        self.writer.write(zeroes);
    }

    fn end_tag() {
        let last_size_pos = vec::pop::<uint>(self.size_positions);
        let cur_pos = self.writer.tell();
        self.writer.seek(last_size_pos as int, io::SeekSet);
        let size = (cur_pos - last_size_pos - 4u);
        write_sized_vuint(self.writer, size, 4u);
        self.writer.seek(cur_pos as int, io::SeekSet);

        debug!("End tag (size = %u)", size);
    }

    fn wr_tag(tag_id: uint, blk: fn()) {
        self.start_tag(tag_id);
        blk();
        self.end_tag();
    }

    fn wr_tagged_bytes(tag_id: uint, b: &[u8]) {
        write_vuint(self.writer, tag_id);
        write_vuint(self.writer, vec::len(b));
        self.writer.write(b);
    }

    fn wr_tagged_u64(tag_id: uint, v: u64) {
        do io::u64_to_be_bytes(v, 8u) |v| {
            self.wr_tagged_bytes(tag_id, v);
        }
    }

    fn wr_tagged_u32(tag_id: uint, v: u32) {
        do io::u64_to_be_bytes(v as u64, 4u) |v| {
            self.wr_tagged_bytes(tag_id, v);
        }
    }

    fn wr_tagged_u16(tag_id: uint, v: u16) {
        do io::u64_to_be_bytes(v as u64, 2u) |v| {
            self.wr_tagged_bytes(tag_id, v);
        }
    }

    fn wr_tagged_u8(tag_id: uint, v: u8) {
        self.wr_tagged_bytes(tag_id, &[v]);
    }

    fn wr_tagged_i64(tag_id: uint, v: i64) {
        do io::u64_to_be_bytes(v as u64, 8u) |v| {
            self.wr_tagged_bytes(tag_id, v);
        }
    }

    fn wr_tagged_i32(tag_id: uint, v: i32) {
        do io::u64_to_be_bytes(v as u64, 4u) |v| {
            self.wr_tagged_bytes(tag_id, v);
        }
    }

    fn wr_tagged_i16(tag_id: uint, v: i16) {
        do io::u64_to_be_bytes(v as u64, 2u) |v| {
            self.wr_tagged_bytes(tag_id, v);
        }
    }

    fn wr_tagged_i8(tag_id: uint, v: i8) {
        self.wr_tagged_bytes(tag_id, &[v as u8]);
    }

    fn wr_tagged_str(tag_id: uint, v: &str) {
        str::byte_slice(v, |b| self.wr_tagged_bytes(tag_id, b));
    }

    fn wr_bytes(b: &[u8]) {
        debug!("Write %u bytes", vec::len(b));
        self.writer.write(b);
    }

    fn wr_str(s: ~str) {
        debug!("Write str: %?", s);
        self.writer.write(str::to_bytes(s));
    }
}

// FIXME (#2743): optionally perform "relaxations" on end_tag to more
// efficiently encode sizes; this is a fixed point iteration

// Set to true to generate more debugging in EBML serialization.
// Totally lame approach.
const debug: bool = false;

enum EbmlSerializerTag {
    EsUint, EsU64, EsU32, EsU16, EsU8,
    EsInt, EsI64, EsI32, EsI16, EsI8,
    EsBool,
    EsStr,
    EsF64, EsF32, EsFloat,
    EsEnum, EsEnumVid, EsEnumBody,
    EsVec, EsVecLen, EsVecElt,

    EsOpaque,

    EsLabel // Used only when debugging
}

priv impl Serializer {
    // used internally to emit things like the vector length and so on
    fn _emit_tagged_uint(t: EbmlSerializerTag, v: uint) {
        assert v <= 0xFFFF_FFFF_u;
        self.wr_tagged_u32(t as uint, v as u32);
    }

    fn _emit_label(label: &str) {
        // There are various strings that we have access to, such as
        // the name of a record field, which do not actually appear in
        // the serialized EBML (normally).  This is just for
        // efficiency.  When debugging, though, we can emit such
        // labels and then they will be checked by deserializer to
        // try and check failures more quickly.
        if debug { self.wr_tagged_str(EsLabel as uint, label) }
    }
}

impl Serializer {
    fn emit_opaque(&self, f: fn()) {
        do self.wr_tag(EsOpaque as uint) {
            f()
        }
    }
}

impl Serializer: serialization2::Serializer {
    fn emit_nil(&self) {}

    fn emit_uint(&self, v: uint) {
        self.wr_tagged_u64(EsUint as uint, v as u64);
    }
    fn emit_u64(&self, v: u64) { self.wr_tagged_u64(EsU64 as uint, v); }
    fn emit_u32(&self, v: u32) { self.wr_tagged_u32(EsU32 as uint, v); }
    fn emit_u16(&self, v: u16) { self.wr_tagged_u16(EsU16 as uint, v); }
    fn emit_u8(&self, v: u8)   { self.wr_tagged_u8 (EsU8  as uint, v); }

    fn emit_int(&self, v: int) {
        self.wr_tagged_i64(EsInt as uint, v as i64);
    }
    fn emit_i64(&self, v: i64) { self.wr_tagged_i64(EsI64 as uint, v); }
    fn emit_i32(&self, v: i32) { self.wr_tagged_i32(EsI32 as uint, v); }
    fn emit_i16(&self, v: i16) { self.wr_tagged_i16(EsI16 as uint, v); }
    fn emit_i8(&self, v: i8)   { self.wr_tagged_i8 (EsI8  as uint, v); }

    fn emit_bool(&self, v: bool) {
        self.wr_tagged_u8(EsBool as uint, v as u8)
    }

    // FIXME (#2742): implement these
    fn emit_f64(&self, _v: f64) { fail ~"Unimplemented: serializing an f64"; }
    fn emit_f32(&self, _v: f32) { fail ~"Unimplemented: serializing an f32"; }
    fn emit_float(&self, _v: float) {
        fail ~"Unimplemented: serializing a float";
    }

    fn emit_str(&self, v: &str) { self.wr_tagged_str(EsStr as uint, v) }

    fn emit_enum(&self, name: &str, f: fn()) {
        self._emit_label(name);
        self.wr_tag(EsEnum as uint, f)
    }
    fn emit_enum_variant(&self, _v_name: &str, v_id: uint, _cnt: uint,
                         f: fn()) {
        self._emit_tagged_uint(EsEnumVid, v_id);
        self.wr_tag(EsEnumBody as uint, f)
    }
    fn emit_enum_variant_arg(&self, _idx: uint, f: fn()) { f() }

    fn emit_vec(&self, len: uint, f: fn()) {
        do self.wr_tag(EsVec as uint) {
            self._emit_tagged_uint(EsVecLen, len);
            f()
        }
    }

    fn emit_vec_elt(&self, _idx: uint, f: fn()) {
        self.wr_tag(EsVecElt as uint, f)
    }

    fn emit_box(&self, f: fn()) { f() }
    fn emit_uniq(&self, f: fn()) { f() }
    fn emit_rec(&self, f: fn()) { f() }
    fn emit_rec_field(&self, f_name: &str, _f_idx: uint, f: fn()) {
        self._emit_label(f_name);
        f()
    }
    fn emit_tup(&self, _sz: uint, f: fn()) { f() }
    fn emit_tup_elt(&self, _idx: uint, f: fn()) { f() }
}

struct Deserializer {
    priv mut parent: Doc,
    priv mut pos: uint,
}

fn Deserializer(d: Doc) -> Deserializer {
    Deserializer { mut parent: d, mut pos: d.start }
}

priv impl Deserializer {
    fn _check_label(lbl: ~str) {
        if self.pos < self.parent.end {
            let TaggedDoc { tag: r_tag, doc: r_doc } =
                doc_at(self.parent.data, self.pos);

            if r_tag == (EsLabel as uint) {
                self.pos = r_doc.end;
                let str = doc_as_str(r_doc);
                if lbl != str {
                    fail fmt!("Expected label %s but found %s", lbl, str);
                }
            }
        }
    }

    fn next_doc(exp_tag: EbmlSerializerTag) -> Doc {
        debug!(". next_doc(exp_tag=%?)", exp_tag);
        if self.pos >= self.parent.end {
            fail ~"no more documents in current node!";
        }
        let TaggedDoc { tag: r_tag, doc: r_doc } =
            doc_at(self.parent.data, self.pos);
        debug!("self.parent=%?-%? self.pos=%? r_tag=%? r_doc=%?-%?",
               copy self.parent.start, copy self.parent.end,
               copy self.pos, r_tag, r_doc.start, r_doc.end);
        if r_tag != (exp_tag as uint) {
            fail fmt!("expected EMBL doc with tag %? but found tag %?",
                      exp_tag, r_tag);
        }
        if r_doc.end > self.parent.end {
            fail fmt!("invalid EBML, child extends to 0x%x, parent to 0x%x",
                      r_doc.end, self.parent.end);
        }
        self.pos = r_doc.end;
        r_doc
    }

    fn push_doc<T>(d: Doc, f: fn() -> T) -> T{
        let old_parent = self.parent;
        let old_pos = self.pos;
        self.parent = d;
        self.pos = d.start;
        let r = f();
        self.parent = old_parent;
        self.pos = old_pos;
        move r
    }

    fn _next_uint(exp_tag: EbmlSerializerTag) -> uint {
        let r = doc_as_u32(self.next_doc(exp_tag));
        debug!("_next_uint exp_tag=%? result=%?", exp_tag, r);
        r as uint
    }
}

impl Deserializer {
    fn read_opaque<R>(&self, op: fn(Doc) -> R) -> R {
        do self.push_doc(self.next_doc(EsOpaque)) {
            op(copy self.parent)
        }
    }
}

impl Deserializer: serialization2::Deserializer {
    fn read_nil(&self) -> () { () }

    fn read_u64(&self) -> u64 { doc_as_u64(self.next_doc(EsU64)) }
    fn read_u32(&self) -> u32 { doc_as_u32(self.next_doc(EsU32)) }
    fn read_u16(&self) -> u16 { doc_as_u16(self.next_doc(EsU16)) }
    fn read_u8 (&self) -> u8  { doc_as_u8 (self.next_doc(EsU8 )) }
    fn read_uint(&self) -> uint {
        let v = doc_as_u64(self.next_doc(EsUint));
        if v > (core::uint::max_value as u64) {
            fail fmt!("uint %? too large for this architecture", v);
        }
        v as uint
    }

    fn read_i64(&self) -> i64 { doc_as_u64(self.next_doc(EsI64)) as i64 }
    fn read_i32(&self) -> i32 { doc_as_u32(self.next_doc(EsI32)) as i32 }
    fn read_i16(&self) -> i16 { doc_as_u16(self.next_doc(EsI16)) as i16 }
    fn read_i8 (&self) -> i8  { doc_as_u8 (self.next_doc(EsI8 )) as i8  }
    fn read_int(&self) -> int {
        let v = doc_as_u64(self.next_doc(EsInt)) as i64;
        if v > (int::max_value as i64) || v < (int::min_value as i64) {
            fail fmt!("int %? out of range for this architecture", v);
        }
        v as int
    }

    fn read_bool(&self) -> bool { doc_as_u8(self.next_doc(EsBool)) as bool }

    fn read_f64(&self) -> f64 { fail ~"read_f64()"; }
    fn read_f32(&self) -> f32 { fail ~"read_f32()"; }
    fn read_float(&self) -> float { fail ~"read_float()"; }

    fn read_str(&self) -> ~str { doc_as_str(self.next_doc(EsStr)) }

    // Compound types:
    fn read_enum<T>(&self, name: ~str, f: fn() -> T) -> T {
        debug!("read_enum(%s)", name);
        self._check_label(name);
        self.push_doc(self.next_doc(EsEnum), f)
    }

    fn read_enum_variant<T>(&self, f: fn(uint) -> T) -> T {
        debug!("read_enum_variant()");
        let idx = self._next_uint(EsEnumVid);
        debug!("  idx=%u", idx);
        do self.push_doc(self.next_doc(EsEnumBody)) {
            f(idx)
        }
    }

    fn read_enum_variant_arg<T>(&self, idx: uint, f: fn() -> T) -> T {
        debug!("read_enum_variant_arg(idx=%u)", idx);
        f()
    }

    fn read_vec<T>(&self, f: fn(uint) -> T) -> T {
        debug!("read_vec()");
        do self.push_doc(self.next_doc(EsVec)) {
            let len = self._next_uint(EsVecLen);
            debug!("  len=%u", len);
            f(len)
        }
    }

    fn read_vec_elt<T>(&self, idx: uint, f: fn() -> T) -> T {
        debug!("read_vec_elt(idx=%u)", idx);
        self.push_doc(self.next_doc(EsVecElt), f)
    }

    fn read_box<T>(&self, f: fn() -> T) -> T {
        debug!("read_box()");
        f()
    }

    fn read_uniq<T>(&self, f: fn() -> T) -> T {
        debug!("read_uniq()");
        f()
    }

    fn read_rec<T>(&self, f: fn() -> T) -> T {
        debug!("read_rec()");
        f()
    }

    fn read_rec_field<T>(&self, f_name: ~str, f_idx: uint,
                         f: fn() -> T) -> T {
        debug!("read_rec_field(%s, idx=%u)", f_name, f_idx);
        self._check_label(f_name);
        f()
    }

    fn read_tup<T>(&self, sz: uint, f: fn() -> T) -> T {
        debug!("read_tup(sz=%u)", sz);
        f()
    }

    fn read_tup_elt<T>(&self, idx: uint, f: fn() -> T) -> T {
        debug!("read_tup_elt(idx=%u)", idx);
        f()
    }
}


// ___________________________________________________________________________
// Testing

#[cfg(test)]
mod tests {
    #[test]
    fn test_option_int() {
        fn test_v(v: Option<int>) {
            debug!("v == %?", v);
            let bytes = do io::with_bytes_writer |wr| {
                let ebml_w = Serializer(wr);
                v.serialize(&ebml_w)
            };
            let ebml_doc = Doc(@bytes);
            let deser = Deserializer(ebml_doc);
            let v1 = serialization2::deserialize(&deser);
            debug!("v1 == %?", v1);
            assert v == v1;
        }

        test_v(Some(22));
        test_v(None);
        test_v(Some(3));
    }
}
