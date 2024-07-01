pub fn main() {
    if !cfg!(feature = "bigtable") {
        panic!("No database defined! Please compile with `features=bigtable`");
    }
}
