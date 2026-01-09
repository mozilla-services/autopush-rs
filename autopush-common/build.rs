pub fn main() {
    if !cfg!(feature = "bigtable") && !cfg!(feature = "redis") {
        panic!("No database defined! Please compile with `features=bigtable` (or redis)");
    }
}
