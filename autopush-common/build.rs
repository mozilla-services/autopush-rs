pub fn main() {
    if !(cfg!(feature = "dynamodb") || cfg!(feature = "bigtable")) {
        panic!("No database defined! Please compile with either `features=dynamodb` or `features=bigtable`");
    }
}
