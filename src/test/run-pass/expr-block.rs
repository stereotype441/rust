// xfail-boot
// -*- rust -*-

// Tests for standalone blocks as expressions

fn test_basic() {
  let bool res = { true };
  check (res);
}

fn test_rec() {
  auto res = { rec(v1 = 10, v2 = 20) };
  check (res.v2 == 20);
}

fn test_filled_with_stuff() {
  auto res = {
    auto a = 0;
    while (a < 10) {
      a += 1;
    }
    a
  };
  check (res == 10);
}

fn main() {
  test_basic();
  test_rec();
  test_filled_with_stuff();
}
