"""A starter Rivers code location: two assets, the second depending on the first.

The Rivers server loads `repo` from this module (the `rivers` module's `module` knob names
it). Edit it, add modules beside it, then reload the code location from the UI. This file
is yours: re-rendering the environment never overwrites it.
"""

import rivers as rs


@rs.Asset
def raw_numbers() -> list[int]:
    return list(range(1, 11))


@rs.Asset
def number_summary(raw_numbers: list[int]) -> dict:
    # The parameter name is the dependency: Rivers materializes `raw_numbers` first.
    return {"count": len(raw_numbers), "total": sum(raw_numbers)}


repo = rs.CodeRepository(assets=[raw_numbers, number_summary])
