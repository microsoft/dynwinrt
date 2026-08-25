import argparse
import importlib
import inspect
import pkgutil

import generated


def import_all_modules() -> int:
    failures: list[tuple[str, Exception]] = []
    modules = sorted(
        module.name
        for module in pkgutil.walk_packages(
            generated.__path__,
            prefix=f"{generated.__name__}.",
        )
    )
    for module_name in modules:
        try:
            importlib.import_module(module_name)
        except Exception as error:
            failures.append((module_name, error))
    if failures:
        details = "\n".join(
            f"  {module}: {type(error).__name__}: {error}"
            for module, error in failures
        )
        raise RuntimeError(f"Generated modules failed to import:\n{details}")
    return len(modules)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--symbol")
    parser.add_argument("--limit", type=int, default=25)
    args = parser.parse_args()

    count = import_all_modules()
    exports = tuple(getattr(generated, "__all__", ()))
    print(f"Imported {count} generated modules.")
    print(f"The package exposes {len(exports)} public symbols.")

    if args.symbol:
        value = getattr(generated, args.symbol)
        try:
            signature = str(inspect.signature(value))
        except (TypeError, ValueError):
            signature = "<not callable>"
        print(f"{args.symbol}: {value!r}")
        print(f"module: {getattr(value, '__module__', None)}")
        print(f"signature: {signature}")
        print(inspect.getdoc(value) or "<no documentation>")
        return

    for name in exports[: args.limit]:
        value = getattr(generated, name)
        print(f"{name}: {getattr(value, '__module__', None)}")


if __name__ == "__main__":
    main()
