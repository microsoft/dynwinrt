# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

from dataclasses import dataclass

from py_runner import projected_values_equal


@dataclass
class ProjectedValue:
    name: str
    value: int


def test_projected_values_equal_compares_all_shared_semantic_attributes():
    left = ProjectedValue(name='shared', value=1)
    right = ProjectedValue(name='shared', value=2)

    assert not projected_values_equal(left, right)


def test_projected_values_equal_retains_scalar_fallback():
    assert projected_values_equal('shared', 'shared')
    assert not projected_values_equal('left', 'right')
