pub fn main() {
    // Clippy freaks out a bit and wants this to be "false"
    #[allow(clippy::nonminimal_bool)]
    if !(cfg!(any(
        feature = "bigtable",
        feature = "postgres",
        feature = "redis"
    ))) {
        panic!("No database defined! Please compile with one of  `features=bigtable`, `features=redis`, `features=postgres`");
    }
}
