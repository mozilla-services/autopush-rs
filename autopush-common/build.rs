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

    // The following is required for `clippy --all --all-features` to pass.
    // Technically, it is an error to have all three backends enabled, and will result
    // in no database being used.
    if cfg!(all(
        feature = "bigtable",
        feature = "postgres",
        feature = "redis"
    )) {
        println!("⚠️ WARNING: Only one database backend is recommended for production use. A default data store will be selected.");
        return;
    }

    if cfg!(any(
        all(feature = "bigtable", feature = "postgres"),
        all(feature = "bigtable", feature = "redis"),
        all(feature = "postgres", feature = "redis")
    )) {
        panic!("Multiple databases selected! Please compile with only one of `features=bigtable`, `features=redis`, `features=postgres`");
    }
}
