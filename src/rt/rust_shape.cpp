// Functions that interpret the shape of a type to perform various low-level
// actions, such as copying, freeing, comparing, and so on.


#include <algorithm>
#include <iomanip>
#include <iostream>
#include <sstream>
#include <utility>

#include "rust_task.h"
#include "rust_shape.h"

namespace shape {

using namespace shape;

// Constants

const uint8_t CMP_EQ = 0u;
const uint8_t CMP_LT = 1u;
const uint8_t CMP_LE = 2u;

// A shape printer, useful for debugging

void
print::walk_tag1(tag_info &tinfo) {
    DPRINT("tag%u", tinfo.tag_id);
}

void
print::walk_struct1(const uint8_t *end_sp) {
    DPRINT("(");

    bool first = true;
    while (sp != end_sp) {
        if (!first)
            DPRINT(",");
        first = false;

        walk();
    }

    DPRINT(")");
}

void
print::walk_res1(const rust_fn *dtor, const uint8_t *end_sp) {
    DPRINT("res@%p", dtor);

    // Print arguments.

    if (sp == end_sp)
        return;

    DPRINT("(");

    bool first = true;
    while (sp != end_sp) {
        if (!first)
            DPRINT(",");
        first = false;

        walk();
    }

    DPRINT(")");
}

template<>
void print::walk_number1<uint8_t>()      { DPRINT("u8"); }
template<>
void print::walk_number1<uint16_t>()     { DPRINT("u16"); }
template<>
void print::walk_number1<uint32_t>()     { DPRINT("u32"); }
template<>
void print::walk_number1<uint64_t>()     { DPRINT("u64"); }
template<>
void print::walk_number1<int8_t>()       { DPRINT("i8"); }
template<>
void print::walk_number1<int16_t>()      { DPRINT("i16"); }
template<>
void print::walk_number1<int32_t>()      { DPRINT("i32"); }
template<>
void print::walk_number1<int64_t>()      { DPRINT("i64"); }
template<>
void print::walk_number1<float>()        { DPRINT("f32"); }
template<>
void print::walk_number1<double>()       { DPRINT("f64"); }


void
size_of::compute_tag_size(tag_info &tinfo) {
    // If the precalculated size and alignment are good, use them.
    if (tinfo.tag_sa.is_set())
        return;

    uint16_t n_largest_variants = get_u16_bump(tinfo.largest_variants_ptr);
    tinfo.tag_sa.set(0, 0);
    for (uint16_t i = 0; i < n_largest_variants; i++) {
        uint16_t variant_id = get_u16_bump(tinfo.largest_variants_ptr);
        std::pair<const uint8_t *,const uint8_t *> variant_ptr_and_end =
            get_variant_sp(tinfo, variant_id);
        const uint8_t *variant_ptr = variant_ptr_and_end.first;
        const uint8_t *variant_end = variant_ptr_and_end.second;

        size_of sub(*this, variant_ptr, NULL);
        sub.align = false;

        // Compute the size of this variant.
        size_align variant_sa;
        bool first = true;
        while (sub.sp != variant_end) {
            if (!first)
                variant_sa.size = align_to(variant_sa.size, sub.sa.alignment);
            sub.walk();
            sub.align = true, first = false;

            variant_sa.add(sub.sa.size, sub.sa.alignment);
        }

        if (tinfo.tag_sa.size < variant_sa.size)
            tinfo.tag_sa = variant_sa;
    }

    if (tinfo.variant_count == 1) {
        if (!tinfo.tag_sa.size)
            tinfo.tag_sa.set(1, 1);
    } else {
        // Add in space for the tag.
        tinfo.tag_sa.add(sizeof(tag_variant_t), rust_alignof<tag_align_t>());
    }
}

void
size_of::walk_tag1(tag_info &tinfo) {
    compute_tag_size(*this, tinfo);
    sa = tinfo.tag_sa;
}

void
size_of::walk_struct1(const uint8_t *end_sp) {
    size_align struct_sa(0, 1);

    bool first = true;
    while (sp != end_sp) {
        if (!first)
            struct_sa.size = align_to(struct_sa.size, sa.alignment);
        walk();
        align = true, first = false;

        struct_sa.add(sa);
    }

    sa = struct_sa;
}

// Copy constructors

#if 0

class copy : public data<copy,uint8_t *> {
    // FIXME #2892
};

#endif


// Structural comparison glue.

class cmp : public data<cmp,ptr_pair> {
    friend class data<cmp,ptr_pair>;

private:
    void walk_slice2(bool is_pod,
                     const std::pair<ptr_pair,ptr_pair> &data_range);

    void walk_vec2(bool is_pod,
                   const std::pair<ptr_pair,ptr_pair> &data_range);

    inline void walk_subcontext2(cmp &sub) {
        sub.walk();
        result = sub.result;
    }

    inline void walk_box_contents2(cmp &sub) {
        sub.align = true;
        sub.walk();
        result = sub.result;
    }

    inline void walk_uniq_contents2(cmp &sub) {
        sub.align = true;
        sub.walk();
        result = sub.result;
    }

    inline void walk_rptr_contents2(cmp &sub) {
        sub.align = true;
        sub.walk();
        result = sub.result;
    }

    inline void cmp_two_pointers() {
        ALIGN_TO(rust_alignof<void *>());
        data_pair<uint8_t *> fst = bump_dp<uint8_t *>(dp);
        data_pair<uint8_t *> snd = bump_dp<uint8_t *>(dp);
        cmp_number(fst);
        if (!result)
            cmp_number(snd);
    }

    inline void cmp_pointer() {
        ALIGN_TO(rust_alignof<void *>());
        cmp_number(bump_dp<uint8_t *>(dp));
    }

    template<typename T>
    void cmp_number(const data_pair<T> &nums) {
        result = (nums.fst < nums.snd) ? -1 : (nums.fst == nums.snd) ? 0 : 1;
    }

public:
    int result;

    cmp(rust_task *in_task,
        bool in_align,
        const uint8_t *in_sp,
        const rust_shape_tables *in_tables,
        uint8_t *in_data_0,
        uint8_t *in_data_1)
    : data<cmp,ptr_pair>(in_task, in_align, in_sp, in_tables,
                         ptr_pair::make(in_data_0, in_data_1)),
      result(0) {}

    cmp(const cmp &other,
        const uint8_t *in_sp,
        const rust_shape_tables *in_tables,
        ptr_pair &in_dp)
    : data<cmp,ptr_pair>(other.task, other.align, in_sp, in_tables,
                         in_dp),
      result(0) {}

    cmp(const cmp &other,
        const uint8_t *in_sp = NULL,
        const rust_shape_tables *in_tables = NULL)
    : data<cmp,ptr_pair>(other.task,
                         other.align,
                         in_sp ? in_sp : other.sp,
                         in_tables ? in_tables : other.tables,
                         other.dp),
      result(0) {}

    cmp(const cmp &other, const ptr_pair &in_dp)
    : data<cmp,ptr_pair>(other.task,
                         other.align,
                         other.sp,
                         other.tables,
                         in_dp),
      result(0) {}

    void walk_vec2(bool is_pod) {
        walk_vec2(is_pod, get_vec_data_range(dp));
    }

    void walk_unboxed_vec2(bool is_pod) {
        walk_vec2(is_pod, get_unboxed_vec_data_range(dp));
    }


    void walk_slice2(bool is_pod, bool is_str) {
        // Slices compare just like vecs.
        walk_vec2(is_pod, get_slice_data_range(is_str, dp));
    }

    void walk_fixedvec2(uint16_t n_elts, size_t elt_sz, bool is_pod) {
        // Fixedvecs compare just like vecs.
        walk_vec2(is_pod, get_fixedvec_data_range(n_elts, elt_sz, dp));
    }

    void walk_box2() {
        data<cmp,ptr_pair>::walk_box_contents1();
    }

    void walk_uniq2() {
        data<cmp,ptr_pair>::walk_uniq_contents1();
    }

    void walk_rptr2() {
        data<cmp,ptr_pair>::walk_rptr_contents1();
    }

    void walk_trait2() {
        data<cmp,ptr_pair>::walk_box_contents1();
    }

    void walk_tydesc2(char) {
        cmp_pointer();
    }

    void walk_fn2(char) { return cmp_two_pointers(); }
    void walk_obj2()    { return cmp_two_pointers(); }

    void walk_tag2(tag_info &tinfo,
                   const data_pair<tag_variant_t> &tag_variants);
    void walk_struct2(const uint8_t *end_sp);
    void walk_res2(const rust_fn *dtor, const uint8_t *end_sp);
    void walk_variant2(tag_info &tinfo,
                       tag_variant_t variant_id,
                       const std::pair<const uint8_t *,const uint8_t *>
                       variant_ptr_and_end);

    template<typename T>
    void walk_number2() { cmp_number(get_dp<T>(dp)); }
};

template<>
void cmp::cmp_number<int32_t>(const data_pair<int32_t> &nums) {
    result = (nums.fst < nums.snd) ? -1 : (nums.fst == nums.snd) ? 0 : 1;
}

void
cmp::walk_vec2(bool is_pod, const std::pair<ptr_pair,ptr_pair> &data_range) {
    cmp sub(*this, data_range.first);
    ptr_pair data_end = sub.end_dp = data_range.second;
    while (!result && sub.dp < data_end) {
        sub.walk_reset();
        result = sub.result;
        sub.align = true;
    }

    if (!result) {
        // If we hit the end, the result comes down to length comparison.
        int len_fst = data_range.second.fst - data_range.first.fst;
        int len_snd = data_range.second.snd - data_range.first.snd;
        cmp_number(data_pair<int>::make(len_fst, len_snd));
    }
}

void
cmp::walk_tag2(tag_info &tinfo,
               const data_pair<tag_variant_t> &tag_variants) {
    cmp_number(tag_variants);
    if (result != 0)
        return;
    data<cmp,ptr_pair>::walk_variant1(tinfo, tag_variants.fst);
}

void
cmp::walk_struct2(const uint8_t *end_sp) {
    while (!result && this->sp != end_sp) {
        this->walk();
        align = true;
    }
}

void
cmp::walk_res2(const rust_fn *dtor, const uint8_t *end_sp) {
    this->cmp_two_pointers();
}

void
cmp::walk_variant2(tag_info &tinfo,
                   tag_variant_t variant_id,
                   const std::pair<const uint8_t *,const uint8_t *>
                   variant_ptr_and_end) {
    cmp sub(*this, variant_ptr_and_end.first);

    const uint8_t *variant_end = variant_ptr_and_end.second;
    while (!result && sub.sp < variant_end) {
        sub.walk();
        result = sub.result;
        sub.align = true;
    }
}


// Polymorphic logging, for convenience

void
log::walk_string2(const std::pair<ptr,ptr> &data) {
    out << prefix << "\"" << std::hex;

    ptr subdp = data.first;
    while (subdp < data.second) {
        char ch = *subdp;
        switch(ch) {
        case '\n': out << "\\n"; break;
        case '\r': out << "\\r"; break;
        case '\t': out << "\\t"; break;
        case '\\': out << "\\\\"; break;
        case '"': out << "\\\""; break;
        default:
            if (isprint(ch)) {
                out << ch;
            } else if (ch) {
                out << "\\x" << std::setw(2) << std::setfill('0')
                    << (unsigned int)(unsigned char)ch;
            }
        }
        ++subdp;
    }

    out << "\"" << std::dec;
}

void
log::walk_struct2(const uint8_t *end_sp) {
    out << prefix << "(";
    prefix = "";

    bool first = true;
    while (sp != end_sp) {
        if (!first)
            out << ", ";
        walk();
        align = true, first = false;
    }

    out << ")";
}

void
log::walk_vec2(bool is_pod, const std::pair<ptr,ptr> &data) {
    if (peek() == SHAPE_U8) {
        sp++;   // It's a string. We handle this ourselves.
        walk_string2(data);
        return;
    }

    out << prefix << "[";

    log sub(*this, data.first);
    sub.end_dp = data.second;

    while (sub.dp < data.second) {
        sub.walk_reset();
        sub.align = true;
        sub.prefix = ", ";
    }

    out << "]";
}

void
log::walk_variant2(tag_info &tinfo,
                   tag_variant_t variant_id,
                   const std::pair<const uint8_t *,const uint8_t *>
                   variant_ptr_and_end) {
    log sub(*this, variant_ptr_and_end.first);
    const uint8_t *variant_end = variant_ptr_and_end.second;

    bool first = true;
    while (sub.sp < variant_end) {
        out << (first ? "(" : ", ");
        sub.walk();
        sub.align = true, first = false;
    }

    if (!first)
        out << ")";
}

void
log::walk_res2(const rust_fn *dtor, const uint8_t *end_sp) {
    out << prefix << "res";

    if (this->sp == end_sp)
        return;

    out << "(";

    bool first = true;
    while (sp != end_sp) {
        if (!first)
            out << ", ";
        walk();
        align = true, first = false;
    }

    out << ")";
}

} // end namespace shape

extern "C" void
shape_cmp_type(int8_t *result, const type_desc *tydesc,
               uint8_t *data_0, uint8_t *data_1, uint8_t cmp_type) {
    rust_task *task = rust_get_current_task();
    shape::arena arena;

    shape::cmp cmp(task, true, tydesc->shape, tydesc->shape_tables,
                   data_0, data_1);
    cmp.walk();

    switch (cmp_type) {
    case shape::CMP_EQ: *result = cmp.result == 0;  break;
    case shape::CMP_LT: *result = cmp.result < 0;   break;
    case shape::CMP_LE: *result = cmp.result <= 0;  break;
    }
}

extern "C" rust_str *
shape_log_str(const type_desc *tydesc, uint8_t *data) {
    rust_task *task = rust_get_current_task();

    shape::arena arena;

    std::stringstream ss;
    shape::log log(task, true, tydesc->shape, tydesc->shape_tables,
                   data, ss);

    log.walk();

    int len = ss.str().length();
    return make_str(task->kernel, ss.str().c_str(), len, "log_str");
}

extern "C" void
shape_log_type(const type_desc *tydesc, uint8_t *data, uint32_t level) {
    rust_task *task = rust_get_current_task();

    shape::arena arena;

    std::stringstream ss;
    shape::log log(task, true, tydesc->shape, tydesc->shape_tables,
                   data, ss);

    log.walk();

    task->sched_loop->get_log().log(task, level, "%s", ss.str().c_str());
}

//
// Local Variables:
// mode: C++
// fill-column: 78;
// indent-tabs-mode: nil
// c-basic-offset: 4
// buffer-file-coding-system: utf-8-unix
// End:
//
