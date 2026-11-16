"""Apply the upstream fix to an isolated copy of the exact published dependency."""

import io
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile
import urllib.request

root = Path(__file__).resolve().parent.parent
target = root / "target"
target.mkdir(exist_ok=True)
directory = Path(tempfile.mkdtemp(prefix="webview-regression-", dir=target))
package = "gpui-fast-0.1.16"
url = f"https://static.crates.io/crates/gpui-fast/{package}.crate"
with urllib.request.urlopen(url, timeout=60) as response:
    data = response.read()
with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
    archive.extractall(directory, filter="data")
source = directory / package
subprocess.run(
    ["git", "apply", "-p3", f"--directory={source.relative_to(root).as_posix()}",
     ".ci/gpui-fast-app-borrow.patch"],
    cwd=root,
    check=True,
)
config = target / "webview-regression.toml"
config.write_text(
    "[patch.crates-io]\ngpui-fast = { path = " + json.dumps(source.as_posix()) + " }\n"
)
print(f"Patched {package}; Cargo override: {config}")
