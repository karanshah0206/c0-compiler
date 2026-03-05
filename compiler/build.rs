extern crate lalrpop;

fn main() {
  lalrpop::Configuration::new()
    .set_out_dir("src")
    .process()
    .unwrap();
}
