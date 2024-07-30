"""
Convert a EC Public key in PEM format into an b64 x962 string.

Autopush will try to scan for known VAPID public keys to track. These keys
are specified in the header as x962 formatted strings. X962 is effectively
"raw" format and contains the two longs that are the coordinates for the
public key.

"""
import base64
import sys

from typing import cast
from cryptography.hazmat.primitives.asymmetric import ec, utils as ec_utils
from cryptography.hazmat.primitives import serialization

try:
    content = open(sys.argv[1], "rb").read()
    pubkey = serialization.load_pem_public_key(content)
except IndexError:
    print ("Please specify a public key PEM file to convert.")
    exit()

pk_string = cast(ec.EllipticCurvePublicKey, pubkey).public_bytes(
    serialization.Encoding.X962,
    serialization.PublicFormat.UncompressedPoint
)

pk_string = base64.urlsafe_b64encode(pk_string).strip(b'=')

print(f"{pk_string.decode()}")
