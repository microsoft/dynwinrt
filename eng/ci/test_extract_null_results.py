# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

import importlib.util
from collections import Counter
from contextlib import redirect_stdout
import io
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = (
    ROOT
    / "tools"
    / "dynwinrt-codegen"
    / "scripts"
    / "extract-null-results.py"
)
SPEC = importlib.util.spec_from_file_location("extract_null_results", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
extractor = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(extractor)


def method_doc(
    api_id: str,
    *,
    returns: str = "The current reading.",
    remarks: str = "",
) -> str:
    return f"""---
-api-id: {api_id}
-api-type: winrt method
---
## -returns
{returns}
## -remarks
{remarks}
"""


class NullResultExtractionTests(unittest.TestCase):
    def apply_override_text(self, text, documented, nullable=()):
        with tempfile.TemporaryDirectory() as directory:
            overrides = Path(directory) / "overrides.txt"
            overrides.write_text(text, encoding="utf-8")
            with patch.object(extractor, "OVERRIDES", overrides):
                return extractor.apply_overrides(set(documented), set(nullable), Counter())

    def test_exact_declaration_alias_retains_documented_class_fact(self):
        source = "M:Windows.Data.Xml.Dom.XmlElement.GetAttributeNode(System.String)"
        target = "M:Windows.Data.Xml.Dom.IXmlElement.GetAttributeNode(System.String)"
        self.assertEqual(
            self.apply_override_text(f"+{source} => {target}", [source]),
            {source, target},
        )
        source = "P:Windows.Data.Xml.Dom.XmlDocument.DocumentElement"
        target = "P:Windows.Data.Xml.Dom.IXmlDocument.DocumentElement"
        self.assertEqual(
            self.apply_override_text(f"+{source} => {target}", [source]),
            {source, target},
        )

    def test_declaration_aliases_reject_malformed_or_drifted_signatures(self):
        source = "M:Windows.Data.Xml.Dom.XmlElement.GetAttributeNode(System.String)"
        target = "M:Windows.Data.Xml.Dom.IXmlElement.GetAttributeNode(System.String)"
        invalid = (
            f"+{source} =>",
            f"+ => {target}",
            f"-{source} => {target}",
            f"+{source} => {source}",
            f"+{source} => {target} => {target}",
            f"+{source} => M:Windows.Data.Xml.Dom.*.GetAttributeNode(System.String)",
            f"+{source} => {target.replace('System.String', 'System.Object')}",
            f"+{source} => {target.replace('GetAttributeNode', 'SetAttributeNode')}",
            f"+{source} => {target.replace('M:', 'P:', 1)}",
            f"+{source} => M:IXmlElement.GetAttributeNode(System.String)",
            f"+{source} => M:Windows.Data.Xml.Dom.IXmlElement[].GetAttributeNode(System.String)",
            f"+{source} => M:Windows.Data.Xml.Dom.IXmlElement.GetAttributeNode",
        )
        for entry in invalid:
            with self.subTest(entry=entry), self.assertRaisesRegex(SystemExit, "exact source => target"):
                self.apply_override_text(entry, [source])

    def test_declaration_alias_requires_exact_documented_source(self):
        source = "M:Windows.Data.Xml.Dom.XmlElement.GetAttributeNode(System.String)"
        target = "M:Windows.Data.Xml.Dom.IXmlElement.GetAttributeNode(System.String)"
        with self.assertRaisesRegex(SystemExit, "matches no documented member"):
            self.apply_override_text(f"+{source} => {target}", [])
        with self.assertRaisesRegex(SystemExit, "matches no documented member"):
            self.apply_override_text(
                f"+{source} => {target}",
                [source.replace("System.String", "System.Object")],
            )

    def test_duplicate_documented_api_id_cannot_supply_an_ambiguous_alias_source(self):
        source = "M:Windows.Data.Xml.Dom.XmlElement.GetAttributeNode(System.String)"
        with (
            patch.object(extractor, "documents", return_value=[method_doc(source)] * 2),
            self.assertRaisesRegex(SystemExit, "duplicate documented api-id"),
        ):
            extractor.extract(Path("."), extractor.COMMIT)

    def test_declaration_aliases_reject_conflicts_and_removals(self):
        source = "P:Windows.Data.Xml.Dom.XmlDocument.DocumentElement"
        other = "P:Contoso.XmlDocument.DocumentElement"
        target = "P:Windows.Data.Xml.Dom.IXmlDocument.DocumentElement"
        for text in (
            f"+{source} => {target}\n+{other} => {target}",
            f"+{source} => {target}\n+{source} => {target}",
            f"-{source}\n+{source} => {target}",
            f"+{source} => {target}\n-{source}",
            f"+{source} => {target}\n-{target}",
        ):
            with self.subTest(text=text), self.assertRaisesRegex(SystemExit, "conflict"):
                self.apply_override_text(text, [source, other])

    def test_pinned_documentation_reproduces_checked_in_table(self):
        cache = os.environ.get("DYNWINRT_WINRT_DOCS_REPO")
        if not cache:
            self.skipTest("set DYNWINRT_WINRT_DOCS_REPO to the local pinned winrt-api git-object cache")
        repo = Path(cache)
        self.assertEqual(extractor.COMMIT, "8448d5eecfbc2ed903f659f350841dcb4888bc8b")
        self.assertEqual(extractor.git(repo, "cat-file", "-t", extractor.COMMIT).strip(), "commit")
        expected = extractor.OUTPUT.read_bytes()
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "windows-null-results.txt"
            for _ in range(2):
                with (
                    patch.object(extractor, "OUTPUT", output),
                    patch("sys.argv", [str(SCRIPT), "--repo", str(repo)]),
                    redirect_stdout(io.StringIO()),
                ):
                    extractor.main()
                self.assertEqual(output.read_bytes(), expected)

    def test_required_return_null_check_is_nullable(self):
        wording = (
            "Before using the return value from this method, the application "
            "must first check that the value is not null. (If the value is "
            "null and you attempt to retrieve it, Windows will generate an "
            "exception.)"
        )
        for sensor in (
            "Accelerometer",
            "Compass",
            "Gyrometer",
            "Inclinometer",
            "LightSensor",
            "OrientationSensor",
        ):
            api_id = f"M:Windows.Devices.Sensors.{sensor}.GetCurrentReading"
            with self.subTest(sensor=sensor):
                self.assertEqual(
                    extractor.classify_document(
                        method_doc(api_id, remarks=wording)
                    ),
                    (api_id, "remarks"),
                )

    def test_negations_arguments_and_null_holders_are_not_nullable(self):
        cases = (
            "This method always returns a value that is not null.",
            (
                "Before using this method, check that the input parameter is "
                "not null."
            ),
            "The returned object contains a JSON null value.",
        )
        api_id = "M:Contoso.Sensor.GetCurrentReading"
        for remarks in cases:
            with self.subTest(remarks=remarks):
                self.assertEqual(
                    extractor.classify_document(
                        method_doc(api_id, remarks=remarks)
                    ),
                    (api_id, None),
                )

    def test_direct_result_nullability_still_uses_result_sections(self):
        method_id = "M:Contoso.Sensor.GetDefault"
        self.assertEqual(
            extractor.classify_document(
                method_doc(
                    method_id,
                    returns="The default sensor, or null if none is installed.",
                )
            ),
            (method_id, "returns"),
        )

        property_id = "P:Contoso.Reading.OptionalValue"
        property_doc = f"""---
-api-id: {property_id}
-api-type: winrt property
---
## -property-value
The current value, or null when no value is available.
"""
        self.assertEqual(
            extractor.classify_document(property_doc),
            (property_id, "property-value"),
        )


if __name__ == "__main__":
    unittest.main()
