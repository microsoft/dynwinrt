import argparse
import asyncio
import re
from pathlib import Path
from typing import cast

from dynwinrt import RoApartment, projected_lifetime_scope
from generated.windows.graphics.imaging import BitmapDecoder
from generated.windows.media.ocr import OcrEngine
from generated.windows.storage import StorageFile
from generated.windows.storage.streams import IRandomAccessStream


def normalized_words(value: str) -> set[str]:
    return set(re.findall(r"[A-Z0-9]+", value.upper()))


async def recognize(path: Path) -> str:
    with RoApartment(1), projected_lifetime_scope():
        file = await StorageFile.get_file_from_path_async(str(path.resolve()))
        if file is None:
            raise RuntimeError("StorageFile returned no image file")
        stream = await file.open_read_async()
        if stream is None:
            raise RuntimeError("StorageFile returned no image stream")

        decoder = await BitmapDecoder.create_async(
            cast(IRandomAccessStream, stream)
        )
        if decoder is None:
            raise RuntimeError("BitmapDecoder returned no decoder")
        bitmap = await decoder.get_software_bitmap_async()
        if bitmap is None:
            raise RuntimeError("BitmapDecoder returned no SoftwareBitmap")

        with bitmap:
            engine = OcrEngine.try_create_from_user_profile_languages()
            if engine is None:
                raise RuntimeError(
                    "No OCR engine is available for the user profile languages"
                )
            result = await engine.recognize_async(bitmap)
            if result is None:
                raise RuntimeError("OcrEngine returned no result")
            return result.text


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--image", type=Path, required=True)
    parser.add_argument(
        "--expect",
        nargs="*",
        default=["DYNWINRT", "OCR", "42"],
    )
    args = parser.parse_args()

    text = asyncio.run(recognize(args.image))
    words = normalized_words(text)
    missing = [word for word in args.expect if word.upper() not in words]
    if missing:
        raise RuntimeError(
            f"OCR result {text!r} did not contain expected words {missing!r}"
        )
    print("python-ocr-ok", text.strip())


if __name__ == "__main__":
    main()
