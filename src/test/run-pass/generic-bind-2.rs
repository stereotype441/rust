// xfail-boot
fn id[T](&T t) -> T {
  ret t;
}

fn main() {
  auto t = tup(1,2,3,4,5,6,7);
  check (t._5 == 6);
  auto f0 = bind id[tup(int,int,int,int,int,int,int)](t);
  check (f0()._5 == 6);
}
