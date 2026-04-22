"""A command-line utility that generates endpoint encryption keys."""
from cryptography.fernet import Fernet


def main():
    key = Fernet.generate_key().decode("utf-8")
    print("\n# Crypto Keys:")
    print(f"AUTOCONNECT__CRYPTO_KEYS=[\"{key}\"]")
    print(f"AUTOEND__CRYPTO_KEYS=[\"{key}\"]")
    print("\n\n# Auth Key:")
    key = Fernet.generate_key().decode("utf-8")
    print(f"AUTOEND__AUTH_KEYS=[\"{key}\"]")


if __name__ == "__main__":
    main()
