import argparse
import hashlib

from dynwinrt import RoApartment, projected_lifetime_scope
from generated.windows.security.cryptography.core import HashAlgorithmProvider
from generated.windows.storage.streams import IBuffer


def sha256(text: str) -> str:
    with RoApartment(1), projected_lifetime_scope():
        provider = HashAlgorithmProvider.open_algorithm("SHA256")
        if provider is None:
            raise RuntimeError("SHA256 provider is unavailable")
        data = IBuffer.from_bytes(text.encode("utf-8"))
        digest = provider.hash_data(data)
        if digest is None:
            raise RuntimeError("HashAlgorithmProvider returned no digest")
        copied_digest = digest.to_bytes()
        expected_length = provider.hash_length
        if len(copied_digest) != expected_length:
            raise RuntimeError(
                f"SHA256 buffer length mismatch: {len(copied_digest)} != "
                f"{expected_length}"
            )
        return copied_digest.hex()


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
