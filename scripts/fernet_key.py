"""
A command-line utility that generates endpoint encryption keys.
"""

from __future__ import print_function
from cryptography.fernet import Fernet


if __name__ == '__main__':
    print("CRYPTO_KEY=\"%s\"" % Fernet.generate_key().decode("UTF-8"))
