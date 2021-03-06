// xfail-stage0
// -*- rust -*-

use std;
import std._str;
import std._vec;


// FIXME: import std.dbg.const_refcount. Currently
// cross-crate const references don't work.
const uint const_refcount = 0x7bad_face_u;

fn fast_growth() {
  let vec[int] v = vec(1,2,3,4,5);
  v += vec(6,7,8,9,0);

  log v.(9);
  check(v.(0) == 1);
  check(v.(7) == 8);
  check(v.(9) == 0);
}

fn slow_growth() {
  let vec[int] v = vec();
  let vec[int] u = v;
  v += vec(17);

  log v.(0);
  check (v.(0) == 17);
}

fn slow_growth2_helper(str s) {   // ref up: s

  obj acc(vec[str] v) {
    fn add(&str s) { v += vec(s); }
  }

  let str ss = s;                 // ref up: s
  let str mumble = "mrghrm";      // ref up: mumble

  {
    /**
     * Within this block, mumble goes into a vec that is referenced
     * both by the local slot v and the acc's v data field.  When we
     * add(s) on the acc, its v undergoes a slow append (allocate a
     * new vec, copy over existing elements).  Here we're testing to
     * see that this slow path goes over well.  In particular, the
     * copy of existing elements should increment the ref count of
     * mumble, the existing str in the originally- shared vec.
     */
    let vec[str] v = vec(mumble); // ref up: v, mumble
    let acc a = acc(v);           // ref up: a, v

    log _vec.refcount[str](v);
    check (_vec.refcount[str](v) == 2u);

    a.add(s);                     // ref up: mumble, s.  ref down: v

    log _vec.refcount[str](v);
    log _str.refcount(s);
    log _str.refcount(mumble);

    check (_vec.refcount[str](v) == 1u);
    check (_str.refcount(s) == const_refcount);
    check (_str.refcount(mumble) == const_refcount);

    log v.(0);
    log _vec.len[str](v);
    check (_str.eq(v.(0), mumble));
    check (_vec.len[str](v) == 1u);
  }                               // ref down: a, mumble, s, v

  log _str.refcount(s);
  log _str.refcount(mumble);

  check (_str.refcount(s) == const_refcount);
  check (_str.refcount(mumble) == const_refcount);

  log mumble;
  log ss;
}                                 // ref down

fn slow_growth2() {
  let str s = "hi";               // ref up: s
  slow_growth2_helper(s);
  log _str.refcount(s);
  check (_str.refcount(s) == const_refcount);
}

fn main() {
  fast_growth();
  slow_growth();
  slow_growth2();
}
