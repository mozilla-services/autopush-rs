pub fn main() {
    if !(cfg!(feature = "bigtable")  || cfg!(feature="postgres")) {
        panic!("No database defined! Please compile with one of  `features=bigtable`, `features=redis`, `features=postgres`");
    }
}
