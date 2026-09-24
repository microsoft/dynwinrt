#!/usr/bin/env python3
# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""Check generated Python packages for names that cannot resolve.

Every generated ``.py`` module must compile, and every global name it reads at
runtime, in any scope including lambdas, must be defined or imported at module
level outside ``if TYPE_CHECKING:`` blocks, or be a builtin. Lazy references
such as ``_dynwinrt_symbol('module', 'Name')`` and relative imports must name a
generated module that defines the name.

Every ``.pyi`` stub must compile, and every name in its annotations (including
quoted forward references) and expressions must be defined or imported.

Usage: python check_generated_python.py PACKAGE_DIR [PACKAGE_DIR ...]
"""

import argparse
import ast
import builtins
import os
import sys

BUILTINS = frozenset(dir(builtins)) | {
    "__builtins__",
    "__class__",
    "__file__",
    "__module__",
    "__path__",
    "__qualname__",
}
LAZY_SYMBOL_HELPERS = frozenset({"_dynwinrt_symbol", "_dynwinrt_enum", "_dynwinrt_wrap_values"})
SCOPE_NODES = (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef, ast.Lambda)
COMPREHENSIONS = (ast.ListComp, ast.SetComp, ast.DictComp, ast.GeneratorExp)


def long_path(path):
    path = os.path.abspath(path)
    if os.name == "nt" and not path.startswith("\\\\?\\"):
        return "\\\\?\\" + path
    return path


def is_type_checking(test):
    return (isinstance(test, ast.Name) and test.id == "TYPE_CHECKING") or (
        isinstance(test, ast.Attribute) and test.attr == "TYPE_CHECKING"
    )


def target_names(target, names):
    if isinstance(target, ast.Name):
        names.add(target.id)
    elif isinstance(target, (ast.Tuple, ast.List)):
        for element in target.elts:
            target_names(element, names)
    elif isinstance(target, ast.Starred):
        target_names(target.value, names)


def expression_bindings(node, names):
    """Walrus targets bind in the enclosing function, even inside comprehensions."""
    for child in ast.walk(node):
        if isinstance(child, ast.NamedExpr):
            target_names(child.target, names)


def statement_bindings(statements, names, type_only=None, module=False):
    """Collect names bound by statements, without entering nested scopes.

    At module level, names bound under ``if TYPE_CHECKING:`` go to ``type_only``.
    """
    for node in statements:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            names.add(node.name)
        elif isinstance(node, ast.Import):
            for alias in node.names:
                names.add(alias.asname or alias.name.split(".")[0])
        elif isinstance(node, ast.ImportFrom):
            for alias in node.names:
                if alias.name != "*":
                    names.add(alias.asname or alias.name)
        elif isinstance(node, ast.Assign):
            for target in node.targets:
                target_names(target, names)
            expression_bindings(node.value, names)
        elif isinstance(node, (ast.AugAssign, ast.AnnAssign)):
            target_names(node.target, names)
        elif isinstance(node, ast.Delete):
            for target in node.targets:
                target_names(target, names)
        elif isinstance(node, (ast.For, ast.AsyncFor)):
            target_names(node.target, names)
            statement_bindings(node.body + node.orelse, names, type_only, module)
        elif isinstance(node, (ast.With, ast.AsyncWith)):
            for item in node.items:
                if item.optional_vars is not None:
                    target_names(item.optional_vars, names)
            statement_bindings(node.body, names, type_only, module)
        elif isinstance(node, ast.If):
            if module and type_only is not None and is_type_checking(node.test):
                statement_bindings(node.body, type_only, None, module)
                statement_bindings(node.orelse, names, type_only, module)
            else:
                statement_bindings(node.body + node.orelse, names, type_only, module)
        elif isinstance(node, ast.While):
            statement_bindings(node.body + node.orelse, names, type_only, module)
        elif isinstance(node, (ast.Try, getattr(ast, "TryStar", ast.Try))):
            statement_bindings(node.body, names, type_only, module)
            for handler in node.handlers:
                if handler.name:
                    names.add(handler.name)
                statement_bindings(handler.body, names, type_only, module)
            statement_bindings(node.orelse + node.finalbody, names, type_only, module)
        elif isinstance(node, (ast.Expr, ast.Return)) and node.value is not None:
            expression_bindings(node.value, names)


def declared(statements, kind):
    names = set()
    for node in statements:
        for child in ast.walk(node):
            if isinstance(child, kind):
                names.update(child.names)
    return names


def function_arguments(arguments):
    names = {argument.arg for argument in arguments.posonlyargs + arguments.args + arguments.kwonlyargs}
    for extra in (arguments.vararg, arguments.kwarg):
        if extra is not None:
            names.add(extra.arg)
    return names


class Scope:
    def __init__(self, kind, bound, globals_=(), nonlocals=()):
        self.kind = kind
        self.bound = set(bound) - set(globals_)
        self.globals = set(globals_)
        self.nonlocals = set(nonlocals)


class Module:
    """Parsed facts about one generated module."""

    def __init__(self, path, source):
        self.path = path
        self.tree = ast.parse(source, filename=path)
        self.runtime_names = set()
        self.type_only_names = set()
        statement_bindings(self.tree.body, self.runtime_names, self.type_only_names, module=True)
        self.star_import = any(
            isinstance(node, ast.ImportFrom) and any(alias.name == "*" for alias in node.names)
            for node in ast.walk(self.tree)
        )
        self.future_annotations = any(
            isinstance(node, ast.ImportFrom)
            and node.module == "__future__"
            and any(alias.name == "annotations" for alias in node.names)
            for node in self.tree.body
        )

    @property
    def all_names(self):
        return self.runtime_names | self.type_only_names


class NameChecker:
    """Resolve name reads through Python's scoping rules."""

    def __init__(self, module, stub, report):
        self.module = module
        self.stub = stub
        self.report = report
        self.module_names = module.all_names if stub else module.runtime_names

    def check(self):
        if self.module.star_import:
            return
        self.body(self.module.tree.body, [], module_level=True)

    def resolve(self, name, scopes, lineno):
        for index, scope in enumerate(reversed(scopes)):
            innermost = index == 0
            if name in scope.globals:
                break
            if scope.kind == "class" and not innermost:
                continue
            if name in scope.bound or name in scope.nonlocals:
                return
        if name in self.module_names or name in BUILTINS:
            return
        if not self.stub and name in self.module.type_only_names:
            self.report(self.module.path, lineno, f"'{name}' is imported only under TYPE_CHECKING")
        else:
            self.report(self.module.path, lineno, f"undefined name '{name}'")

    def body(self, statements, scopes, module_level=False):
        for node in statements:
            if module_level and not self.stub and isinstance(node, ast.If) and is_type_checking(node.test):
                # Not executed at runtime; the imported names serve type checkers.
                self.body(node.orelse, scopes, module_level)
                continue
            self.statement(node, scopes)

    def annotation(self, node, scopes):
        if node is None:
            return
        if self.stub:
            self.type_expression(node, scopes)
        elif not self.module.future_annotations:
            self.expression(node, scopes)

    def type_expression(self, node, scopes):
        """Check a stub annotation, including quoted forward references."""
        if isinstance(node, ast.Constant) and isinstance(node.value, str):
            try:
                parsed = ast.parse(node.value, mode="eval").body
            except SyntaxError:
                self.report(self.module.path, node.lineno, f"invalid forward reference {node.value!r}")
                return
            ast.increment_lineno(parsed, node.lineno - 1)
            self.type_expression(parsed, scopes)
        elif isinstance(node, ast.Subscript) and (
            (isinstance(node.value, ast.Name) and node.value.id == "Literal")
            or (isinstance(node.value, ast.Attribute) and node.value.attr == "Literal")
        ):
            self.expression(node.value, scopes)
        elif isinstance(node, ast.Name):
            self.resolve(node.id, scopes, node.lineno)
        else:
            for child in ast.iter_child_nodes(node):
                if isinstance(child, ast.expr):
                    self.type_expression(child, scopes)

    def arguments(self, arguments, scopes):
        for default in arguments.defaults + [value for value in arguments.kw_defaults if value is not None]:
            self.expression(default, scopes)
        for argument in arguments.posonlyargs + arguments.args + arguments.kwonlyargs:
            self.annotation(argument.annotation, scopes)
        for extra in (arguments.vararg, arguments.kwarg):
            if extra is not None:
                self.annotation(extra.annotation, scopes)

    def statement(self, node, scopes):
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            for decorator in node.decorator_list:
                self.expression(decorator, scopes)
            self.arguments(node.args, scopes)
            self.annotation(node.returns, scopes)
            bound = function_arguments(node.args)
            statement_bindings(node.body, bound)
            scope = Scope("function", bound, declared(node.body, ast.Global), declared(node.body, ast.Nonlocal))
            self.body(node.body, scopes + [scope])
        elif isinstance(node, ast.ClassDef):
            for expression in node.decorator_list + node.bases + [keyword.value for keyword in node.keywords]:
                (self.type_expression if self.stub else self.expression)(expression, scopes)
            bound = set()
            statement_bindings(node.body, bound)
            self.body(node.body, scopes + [Scope("class", bound, declared(node.body, ast.Global))])
        elif isinstance(node, ast.AnnAssign):
            self.annotation(node.annotation, scopes)
            if node.value is not None:
                self.expression(node.value, scopes)
            self.target(node.target, scopes)
        elif isinstance(node, ast.AugAssign):
            if isinstance(node.target, ast.Name):
                self.resolve(node.target.id, scopes, node.lineno)
            else:
                self.target(node.target, scopes)
            self.expression(node.value, scopes)
        elif isinstance(node, ast.Assign):
            self.expression(node.value, scopes)
            for target in node.targets:
                self.target(target, scopes)
        elif isinstance(node, ast.Delete):
            for target in node.targets:
                self.target(target, scopes)
        elif isinstance(node, (ast.For, ast.AsyncFor)):
            self.expression(node.iter, scopes)
            self.target(node.target, scopes)
            self.body(node.body + node.orelse, scopes)
        elif isinstance(node, (ast.With, ast.AsyncWith)):
            for item in node.items:
                self.expression(item.context_expr, scopes)
                if item.optional_vars is not None:
                    self.target(item.optional_vars, scopes)
            self.body(node.body, scopes)
        elif isinstance(node, (ast.If, ast.While)):
            self.expression(node.test, scopes)
            self.body(node.body + node.orelse, scopes)
        elif isinstance(node, (ast.Try, getattr(ast, "TryStar", ast.Try))):
            self.body(node.body, scopes)
            for handler in node.handlers:
                if handler.type is not None:
                    self.expression(handler.type, scopes)
                self.body(handler.body, scopes)
            self.body(node.orelse + node.finalbody, scopes)
        elif isinstance(node, (ast.Import, ast.ImportFrom, ast.Global, ast.Nonlocal, ast.Pass, ast.Break, ast.Continue)):
            return
        else:
            for child in ast.iter_child_nodes(node):
                if isinstance(child, ast.expr):
                    self.expression(child, scopes)
                elif isinstance(child, ast.stmt):
                    self.statement(child, scopes)

    def target(self, node, scopes):
        """Visit the reads inside an assignment target."""
        if isinstance(node, (ast.Tuple, ast.List)):
            for element in node.elts:
                self.target(element, scopes)
        elif isinstance(node, ast.Starred):
            self.target(node.value, scopes)
        elif isinstance(node, (ast.Attribute, ast.Subscript)):
            self.expression(node, scopes)

    def expression(self, node, scopes):
        if isinstance(node, ast.Name):
            if isinstance(node.ctx, ast.Load):
                self.resolve(node.id, scopes, node.lineno)
        elif isinstance(node, ast.Lambda):
            self.arguments(node.args, scopes)
            bound = function_arguments(node.args)
            expression_bindings(node.body, bound)
            self.expression(node.body, scopes + [Scope("function", bound)])
        elif isinstance(node, COMPREHENSIONS):
            generators = node.generators
            self.expression(generators[0].iter, scopes)
            bound = set()
            for generator in generators:
                target_names(generator.target, bound)
            inner = scopes + [Scope("function", bound)]
            for index, generator in enumerate(generators):
                if index:
                    self.expression(generator.iter, inner)
                for condition in generator.ifs:
                    self.expression(condition, inner)
            if isinstance(node, ast.DictComp):
                self.expression(node.key, inner)
                self.expression(node.value, inner)
            else:
                self.expression(node.elt, inner)
        else:
            for child in ast.iter_child_nodes(node):
                if isinstance(child, ast.expr):
                    self.expression(child, scopes)
                elif isinstance(child, ast.keyword):
                    self.expression(child.value, scopes)


class PackageChecker:
    def __init__(self, root):
        self.root = long_path(root)
        self.modules = {}
        self.problems = []

    def report(self, path, lineno, message):
        self.problems.append(f"{os.path.relpath(path, self.root)}:{lineno}: {message}")

    def load(self, path):
        if path not in self.modules:
            try:
                with open(path, encoding="utf-8") as handle:
                    source = handle.read()
                compile(source, path, "exec", dont_inherit=True)
                self.modules[path] = Module(path, source)
            except SyntaxError as error:
                self.report(path, error.lineno or 0, f"cannot compile: {error.msg}")
                self.modules[path] = None
        return self.modules[path]

    def module_file(self, directory, dotted, stub):
        """The generated file for a module named relative to `directory`."""
        base = os.path.join(directory, *dotted.split(".")) if dotted else directory
        suffixes = (".pyi", ".py") if stub else (".py",)
        for candidate in [base + suffix for suffix in suffixes] + [
            os.path.join(base, "__init__" + suffix) for suffix in suffixes
        ]:
            if os.path.isfile(candidate):
                return candidate
        return None

    def check_import(self, path, node, stub):
        directory = os.path.dirname(path)
        for _ in range(node.level - 1):
            directory = os.path.dirname(directory)
        target = self.module_file(directory, node.module or "", stub)
        if target is None:
            self.report(path, node.lineno, f"relative import of missing module '{'.' * node.level}{node.module or ''}'")
            return
        module = self.load(target)
        if module is None or module.star_import:
            return
        names = module.all_names if stub else module.runtime_names
        for alias in node.names:
            if alias.name == "*" or alias.name in names:
                continue
            if node.module is None and self.module_file(directory, alias.name, stub):
                continue
            self.report(path, node.lineno, f"'{alias.name}' is not defined by {os.path.relpath(target, self.root)}")

    def check_imports(self, path, module, stub):
        type_checking_imports = set()
        for node in module.tree.body:
            if isinstance(node, ast.If) and is_type_checking(node.test):
                type_checking_imports.update(
                    id(child) for child in ast.walk(node) if isinstance(child, ast.ImportFrom)
                )
        for node in ast.walk(module.tree):
            if isinstance(node, ast.ImportFrom) and node.level:
                # TYPE_CHECKING imports serve type checkers, which read stubs first.
                self.check_import(path, node, stub or id(node) in type_checking_imports)

    def check_exports(self, path, module):
        """Facade `_EXPORTS` maps names to `('.module', 'symbol')` lazy imports."""
        for node in module.tree.body:
            if not (
                isinstance(node, ast.Assign)
                and any(isinstance(target, ast.Name) and target.id == "_EXPORTS" for target in node.targets)
                and isinstance(node.value, ast.Dict)
            ):
                continue
            for value in node.value.values:
                if not (
                    isinstance(value, ast.Tuple)
                    and len(value.elts) == 2
                    and all(isinstance(item, ast.Constant) and isinstance(item.value, str) for item in value.elts)
                ):
                    continue
                relative, symbol = value.elts[0].value, value.elts[1].value
                level = len(relative) - len(relative.lstrip("."))
                self.check_import(
                    path,
                    ast.ImportFrom(
                        module=relative[level:] or None,
                        names=[ast.alias(name=symbol)],
                        level=level,
                        lineno=value.lineno,
                    ),
                    stub=False,
                )

    def check_lazy_symbols(self, path, module):
        for node in ast.walk(module.tree):
            if not (
                isinstance(node, ast.Call)
                and isinstance(node.func, ast.Name)
                and node.func.id in LAZY_SYMBOL_HELPERS
                and len(node.args) >= 2
                and all(isinstance(arg, ast.Constant) and isinstance(arg.value, str) for arg in node.args[:2])
            ):
                continue
            module_name, symbol = node.args[0].value, node.args[1].value
            target = self.module_file(self.root, module_name, stub=False)
            if target is None:
                self.report(path, node.lineno, f"{node.func.id} names missing module '{module_name}'")
                continue
            target_module = self.load(target)
            if target_module is not None and not target_module.star_import and symbol not in target_module.runtime_names:
                self.report(path, node.lineno, f"{node.func.id} names '{symbol}', which {module_name}.py does not define")

    def check(self):
        for directory, _, names in os.walk(self.root):
            for name in sorted(names):
                if not name.endswith((".py", ".pyi")):
                    continue
                path = os.path.join(directory, name)
                stub = name.endswith(".pyi")
                module = self.load(path)
                if module is None:
                    continue
                NameChecker(module, stub, self.report).check()
                self.check_imports(path, module, stub)
                if not stub:
                    self.check_lazy_symbols(path, module)
                    if name == "__init__.py":
                        self.check_exports(path, module)
        return sorted(set(self.problems))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("packages", nargs="+", help="generated Python package directories")
    parser.add_argument("--limit", type=int, default=200, help="maximum problems to print")
    args = parser.parse_args()

    problems = []
    for package in args.packages:
        if not os.path.isfile(os.path.join(long_path(package), "_runtime.py")):
            print(f"{package}: not a generated Python package (no _runtime.py)", file=sys.stderr)
            return 2
        problems.extend(f"{package}: {problem}" for problem in PackageChecker(package).check())
    for problem in problems[: args.limit]:
        print(problem)
    if len(problems) > args.limit:
        print(f"... {len(problems) - args.limit} more")
    print(f"Checked {len(args.packages)} generated Python package(s): {len(problems)} problem(s).")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
