// xfail-boot
// -*- rust -*-

// Tests for if as expressions returning boxed types

fn test_box() {
  auto res = if (true) { @100 } else { @101 };
  check (*res == 100);
}

fn test_str() {
  auto res = if (true) { "happy" } else { "sad" };
  check (res == "happy");
}

fn main() {
  test_box();
  test_str();
}
