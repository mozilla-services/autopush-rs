# VAPID key storage directory

This directory should contain a `private_key.pem` file that matches at
least one of the `AUTOEND_TRACKING_KEYS`. Note that
`AUTOEND_TRACKING_KEYS` uses a x962 formatted public key string.

## Generating test values

`py-vapid` can create a set of valid VAPID keys, provided it's installed into the active virtual environment

```bash
vapid --gen
```

To generate an x962 formatted version of the public key, use `$PROJECT_ROOT/scripts/convert_pem_to_x692.py`

```bash
scripts/convert_pem_to_x692.py tests/load/keys/public_key.pem > tests/load/keys/public_key.x692
```

## Running

By default, the locust builder will include the contents of `$PROJECT_ROOT/test/load/keys`.
Note that locust will reuse docker images when possible. If you change or add keys, you will need to remove the
`locust` container and images in order for these to be rebuilt and the new keys included.

Remove the key from the **Custom parameters**::Vapid Key field to skip load testing the reliability system in addition to the regular load testing.
