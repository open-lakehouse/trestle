"""A oneof on a request: one optional keyword argument per variant, at most one set."""

import asyncio
import json

from demo._core import DemoClient
from demo._core.demo import Label

# The server's URL, which `run_python` in src/tests.rs sets.
BASE: str = globals()["BASE"]

client = DemoClient(BASE)


def sent(response) -> dict:
    """The JSON body the server received."""
    return json.loads(response.body)


# Each variant reaches the server as its own field; a keyword-named one keeps its proto name.
assert sent(client.point_widget("w", ref="other")) == {"ref": "other"}
assert sent(client.point_widget("w", seq=7)) == {"seq": 7}
assert sent(client.point_widget("w", label=Label(key="k", value="v"))) == {
    "label": {"key": "k", "value": "v"}
}
# None set leaves the oneof out.
assert sent(client.point_widget("w")) == {}

# More than one set is a ValueError, before any request is sent.
for kwargs in ({"ref": "a", "seq": 1}, {"seq": 1, "label": Label(key="k")}):
    try:
        client.point_widget("w", **kwargs)
        raise AssertionError(f"{kwargs} should be rejected")
    except ValueError as e:
        assert str(e) == "at most one of `ref`, `seq`, `label` may be set", e


async def main() -> None:
    assert sent(await client.point_widget_async("w", seq=3)) == {"seq": 3}
    try:
        await client.point_widget_async("w", ref="a", seq=1)
        raise AssertionError("two variants should be rejected")
    except ValueError:
        pass


asyncio.run(main())
