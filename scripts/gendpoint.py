#! env python3

import argparse
import os

from cryptography.fernet import Fernet


def config(env_args: os._Environ) -> argparse.Namespace:
    """Read the configuration from the args and environment."""
    parser = argparse.ArgumentParser(
        description="Generate Endpoint from environment and known UAID + CHID")
    parser.add_argument(
        '--key', '-k',
        help="Endpoint cryptographic keys. Can be either a single key "
             "string or a list of strings. Will only use the first key"
             " found.",
        default=env_args.get("AUTOEND_CRYPTO_KEYS"))
    parser.add_argument(
        '--uaid', '-u', required=True,
        help="User Agent ID")
    parser.add_argument(
        '--chid', '-c', required=True,
        help="Channel ID")
    parser.add_argument(
        '--root', '-r',
        default=env_args.get("AUTOEND_ENDPOINT_URL"),
        help="Server root (e.g. 'example.com')")
    args = parser.parse_args()
    if not args.key:
        raise Exception("Missing key")
    if not args.root:
        raise Exception("Missing root")
    return args


def main():
    args = config(os.environ)
    if isinstance(args.key, list):
        key = args.key[0]
    else:
        key = args.key
    print(gendpoint(key, args))


def gendpoint(key: bytes, args: argparse.Namespace) -> str:
    """Generate the endpoint"""
    fernet = Fernet(key)
    base = bytes(
        list((bytes.fromhex(args.uaid.replace('-', '')) +
              bytes.fromhex(args.chid.replace('-', '')))))
    ekey = fernet.encrypt(base).strip(b'=').decode('utf-8')
    return f"https://{args.root}/wpush/v1/{ekey}"


main()
