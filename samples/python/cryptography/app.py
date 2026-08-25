import argparse
import hashlib

from dynwinrt import RoApartment, projected_lifetime_scope
from generated.windows.security.cryptography import (
    BinaryStringEncoding,
    CryptographicBuffer,
)
from generated.windows.security.cryptography.core import HashAlgorithmProvider


def sha256(text: str) -> str:
    with RoApartment(1), projected_lifetime_scope():
        provider = HashAlgorithmProvider.open_algorithm("SHA256")
        if provider is None:
            raise RuntimeError("SHA256 provider is unavailable")
        data = CryptographicBuffer.convert_string_to_binary(
            text,
            BinaryStringEncoding.Utf8,
        )
        if data is None:
            raise RuntimeError("CryptographicBuffer returned no input buffer")
        digest = provider.hash_data(data)
        if digest is None:
            raise RuntimeError("HashAlgorithmProvider returned no digest")
        return CryptographicBuffer.encode_to_hex_string(digest).lower()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--text", default="Hello from dynwinrt.")
    args = parser.parse_args()

    actual = sha256(args.text)
    expected = hashlib.sha256(args.text.encode("utf-8")).hexdigest()
    if actual != expected:
        raise RuntimeError(f"SHA256 mismatch: {actual} != {expected}")
    print("python-cryptography-ok", actual)


if __name__ == "__main__":
    main()
