#! /bin/bash
# Generate a pair of certificate requests.
# These will use the content of the `apns.toml` file to create
# requests for mozilla. You only need the *.csr file, but will
# need to upload that to Apple.
#
# It's recommended that you generate this request _before_ you
# begin the cert request process. You only need one request for
# all the keys in the batch.
#
DATE=$(date +"%Y_%b_%d")
SSL_CMD=openssl
CONFIG=apns.toml
SERVER_KEY=${DATE}_APNS_PRIV.key
CERT_FILE=${DATE}_APNS.csr
echo Generating the private server key: $SERVER_KEY
# Apple only accepts RSA 2048 keys
$SSL_CMD genrsa -out $SERVER_KEY 2048
echo Generating cert file: $CERT_FILE
$SSL_CMD req -new -key $SERVER_KEY -out $CERT_FILE -config $CONFIG
echo Cert contents
$SSL_CMD req -in $CERT_FILE -noout -text
