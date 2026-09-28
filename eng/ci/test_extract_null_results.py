# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

import importlib.util
from pathlib import Path
import unittest


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
