# Various things we learned while dealing with Bigtable

This document contains tricks and traps that we've discovered while working with Bigtable.

## Indicies

* Bigtable uses a single key, which it uses to locate data as proximate to other values (e.g.
"Foo-123" would be stored proximate to "Foo-456"). The suggested mechanism is to collapse significant values into a single key using well-known separator values and use regular-expression syntax for the key index to limit searches.

* Regular expressions must match the entire key. A partial match is not considered for inclusion in the result set. (e.g. `Foo` will not match `Foo-123`, however `Foo.*` will)

## Cells

* Cell values are stored as Byte arrays, and library functions use field hints in order to store and retrieve values, but be mindful of storage differences. (e.g. a u64 value or search will not match a u128 byte array.)

## Filters

* Cell Filters are exclusive by default. This means that when you do a filter for a given cell value or qualifier, the result set will only contain the filtered results.
This is a bit hard to explain without a proper example, but presume the following filter:

```rust
fn expiry_filter() -> Result<data::RowFilter, error::BigTableError> {
    let mut expiry_filter = data::RowFilter::default();
    let mut chain = data::RowFilter_Chain::default();
    let mut filter_chain = RepeatedField::default();

    let mut key_filter = data::RowFilter::default();
    key_filter.set_column_qualifier_regex_filter("expiry".as_bytes().to_vec());
    let bt_now: u128 = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(error::BigTableError::WriteTime)?
        .as_millis();
    let mut value_filter = data::RowFilter::default();
    let mut range = data::ValueRange::default();

    // Valid timestamps have not yet expired.
    range.set_start_value_open(bt_now.to_be_bytes().to_vec());
    value_filter.set_value_range_filter(range);
    filter_chain.push(key_filter);
    filter_chain.push(value_filter);
    chain.set_filters(filter_chain);
    expiry_filter.set_chain(chain);
    Ok(expiry_filter)
}

```

Adding this filter to a query will return a result that only includes the "expiry" cells which have a value that is greater than the current time. No other cells or values will be included in the return set, however each "row" will include the row meta information, including the key.

## Connections

GRPC is not very smart. It's often best to include the port number when including the DSN (e.g. `grpc://bigtable.googleapis.com:443`)