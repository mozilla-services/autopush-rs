#!/bin/bash
# Convert a .cer/.key file into a secret that can be included in the secrets file
IN=$1
openssl x509 -in $IN | awk 1 ORS='\\n' > $IN.secret
