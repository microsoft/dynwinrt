import asyncio
import tempfile
from pathlib import Path

from dynwinrt import RoApartment, projected_lifetime_scope
from generated.windows.storage import (
    CreationCollisionOption,
    FileIO,
    IStorageFile,
    StorageFolder,
)


async def run() -> None:
    expected = "Hello from dynwinrt.\nAsync WinRT file I/O works."

    with tempfile.TemporaryDirectory(prefix="dynwinrt-python-") as directory:
        with RoApartment(1), projected_lifetime_scope():
            folder = await StorageFolder.get_folder_from_path_async(directory)
            if folder is None:
                raise RuntimeError("StorageFolder returned no temporary folder")

            file = await folder.create_file_async(
                "sample.txt",
                CreationCollisionOption.ReplaceExisting,
            )
            if file is None:
                raise RuntimeError("StorageFolder returned no file")
            storage_file = file.as_interface(IStorageFile)

            await FileIO.write_text_async(storage_file, "Hello from dynwinrt.")
            await FileIO.append_text_async(
                storage_file,
                "\nAsync WinRT file I/O works.",
            )
            actual = await FileIO.read_text_async(storage_file)
            if actual != expected:
                raise RuntimeError(
                    f"Unexpected file contents: {actual!r}"
                )

            print(
                "python-file-io-ok",
                {
                    "path": str(Path(directory) / "sample.txt"),
                    "characters": len(actual),
                },
            )


if __name__ == "__main__":
    asyncio.run(run())
