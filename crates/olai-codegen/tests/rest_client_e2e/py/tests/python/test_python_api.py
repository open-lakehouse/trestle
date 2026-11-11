"""The Python API of the generated client: models, enums, errors, JSON fields,
raw bodies, streams and the `_async` methods."""

import asyncio
import pickle

from demo._core import (
    DemoClient,
    DemoError,
    NotFoundError,
    PermissionDeniedError,
    ServerError,
)
from demo._core.demo import Label, Widget, WidgetClient
from demo._enums import Color

# The server's URL, which `run_python` in src/tests.rs sets.
BASE: str = globals()["BASE"]

client = DemoClient(BASE)

# Module paths.
assert DemoClient.__module__ == "demo._core", DemoClient.__module__
assert Widget.__module__ == "demo._core.demo", Widget.__module__
assert WidgetClient.__module__ == "demo._core.demo", WidgetClient.__module__

# Frozen models with a Python repr; an unknown enum value reads as None.
w = client.widget("a/b c").get()
assert w.name == "a/b c"
assert w.color is None, w.color
assert w.created_at == 1700000000000
assert w.properties == {"a": "1"}
assert repr(w) == (
    "Widget(name='a/b c', created_at=1700000000000, size_bytes=0, "
    "properties={'a': '1'}, display_name='', metadata_location='', notes=[], rows={}, "
    "sizes=[], counts={}, labels_by_key={})"
), repr(w)
try:
    w.name = "x"
    raise AssertionError("models are frozen")
except AttributeError:
    pass

# Enum members are their names as str; parameters take a member or a name.
g = client.widget("green").get()
assert g.color is Color.GREEN
assert g.color == "GREEN" and isinstance(g.color, str) and str(g.color) == "GREEN"
assert f"{g.color}" == "GREEN"
assert [x.name for x in client.list_widgets(color="GREEN")] == ["g1"]
assert [x.name for x in client.list_widgets(color=Color.GREEN)] == ["g1"]
assert not hasattr(Color, "COLOR_UNSPECIFIED")
try:
    client.list_widgets(color="BLUE")
    raise AssertionError("unknown enum names are rejected")
except ValueError as e:
    assert "expected one of: RED, GREEN" in str(e), e

# Lists read every page.
assert [x.name for x in client.list_widgets()] == ["w1", "w2", "w3"]

# Only input models have a constructor; optional arguments are keyword-only.
label = Label(key="k", value="v")
assert Label("k").key == "k"
# Value equality; a `debug_redact` field is hidden in the repr only.
assert Label("k") == Label("k") and Label("k") != Label("j")
hidden = Label("k", secret="s3cret")
assert hidden.secret == "s3cret" and "s3cret" not in repr(hidden), repr(hidden)
assert repr(hidden) == "Label(key='k', value='', secret='***')", repr(hidden)
try:
    Label(value="v")
    raise AssertionError("expected a TypeError for the missing key")
except TypeError:
    pass
assert label.key == "k"
try:
    Widget(name="x")
    raise AssertionError("a response model has no constructor")
except TypeError:
    pass
created = client.create_widget("n", created_at=5, labels=[label])
assert created.created_at == 5
try:
    client.create_widget("n", 5)
    raise AssertionError("optional arguments are keyword-only")
except TypeError:
    pass

# Error answers raise the exception class of their status.
try:
    client.widget("missing").get()
    raise AssertionError("expected an error")
except NotFoundError as e:
    assert isinstance(e, DemoError) and isinstance(e, Exception)
    assert type(e).__module__ == "demo._core", type(e).__module__
    assert e.code == "WIDGET_NOT_FOUND" and e.status == 404, (e.code, e.status)
    assert e.details == [{"reason": "gone"}], e.details
    assert str(e) == "WIDGET_NOT_FOUND: no widget named missing", str(e)
    assert (
        repr(e)
        == "NotFoundError('no widget named missing', code='WIDGET_NOT_FOUND', status=404)"
    ), repr(e)
    # Pickle round-trip, as multiprocessing needs.
    again = pickle.loads(pickle.dumps(e))
    assert (
        type(again) is NotFoundError
        and str(again) == str(e)
        and again.details == e.details
    )
    assert e.args == ("no widget named missing",), e.args
# Users can raise them too.
try:
    raise ServerError("down", status=503)
except DemoError as e:
    assert str(e) == "HTTP 503: down" and e.details == [], (str(e), e.details)
    assert e.args == ("down",), e.args
try:
    DemoClient("http://127.0.0.1:9/").widget("w").get()
    raise AssertionError("expected an error")
except DemoError as e:
    assert type(e) is DemoError and e.status is None and e.code is None
    # The message includes the Rust cause chain.
    assert "onnect" in str(e).split("error sending request", 1)[1], str(e)

# Free-form JSON fields are plain Python objects; whole numbers are ints both ways.
w = client.widget("json").get()
assert w.attributes == {"count": 3, "ratio": 0.5, "nested": {"id": 7}}, w.attributes
assert type(w.attributes["count"]) is int
assert w.extra == [1, "two", None, True], w.extra
assert w.notes == [{"a": 1}, 2.5], w.notes
assert w.rows == {"t": [1, 2]}, w.rows
created = client.create_widget(
    "w2", attributes={"count": 3, "flag": True, "list": [1, 2.5]}
)
assert created.attributes == {"count": 3, "flag": True, "list": [1, 2.5]}, (
    created.attributes
)
assert '"count":3' in created.display_name, created.display_name
# A field named after a Rust keyword.
assert client.create_widget("w4", type="gear").type == "gear"
# A map of messages.
by_key = client.create_widget("w5", labels_by_key={"k": label}).labels_by_key
assert list(by_key) == ["k"] and by_key["k"].value == "v", by_key
# A list of free-form JSON.
assert client.create_widget("w6", notes=[1, "two"]).notes == [1, "two"]
try:
    client.create_widget("w3", attributes={"bad": object()})
    raise AssertionError("expected a TypeError")
except TypeError:
    pass
# JSON has no NaN or infinity.
for bad in (float("nan"), float("inf")):
    try:
        client.create_widget("w3", attributes={"bad": bad})
        raise AssertionError("expected a ValueError")
    except ValueError:
        pass

# A HEAD existence check, and a GET with a JSON body.
assert client.widget_exists("w") is True
assert client.widget_exists("missing") is False
try:
    client.widget_exists("denied")
    raise AssertionError("expected an error")
except PermissionDeniedError:
    pass
assert [w.name for w in client.search_widgets("w", ["a"], {"t": [1, 2]}).widgets] == [
    "w1",
    "w2",
]

# An NDJSON stream as a list; fields from response headers.
assert [w.name for w in client.stream_widgets()] == ["w1", "w2"]
version = client.widget_version("w")
assert (version.name, version.version, version.etag) == ("w", 7, "abc"), version

# Raw bodies as `bytes`.
assert client.put_blob("x/y", b"\x00\x01\xff") is None
blob = client.get_blob("x/y")
assert (blob.data, blob.size) == (b"blob:x/y", 8), blob
assert client.get_blob("x/y", range="bytes=0-1").data == b"range:bytes=0-1"


# Each method has an `_async` twin.
async def main():
    w = await client.widget("w").get_async()
    assert w.name == "w"
    assert await client.widget("w").delete_async() is None
    names = [x.name for x in await client.list_widgets_async()]
    assert names == ["w1", "w2", "w3"], names
    try:
        await client.widget("missing").get_async()
        raise AssertionError("expected an error")
    except NotFoundError:
        pass


asyncio.run(main())
