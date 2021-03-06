// xfail-boot
// xfail-stage0
// -*- rust -*-

type compare[T] = fn(&T t1, &T t2) -> bool;

fn test_generic[T](&T expected, &compare[T] eq) {
  let T actual = { expected };
  check (eq(expected, actual));
}

fn test_vec() {
  fn compare_vec(&vec[int] v1, &vec[int] v2) -> bool {
    ret v1 == v2;
  }
  auto eq = bind compare_vec(_, _);
  test_generic[vec[int]](vec(1, 2), eq);
}

fn main() {
  test_vec();
}