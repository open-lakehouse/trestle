// Resets the tarball URL in each bun.lock entry to "", Bun's value for "the
// default registry". Local installs may go through a private npm mirror (e.g.
// a corporate proxy from ~/.npmrc), and Bun records its URLs, which GitHub
// runners can't reach: a frozen install there hangs. Runs after every install
// (postinstall); CI checks the committed lock is clean before installing.
const lock = new URL("../bun.lock", import.meta.url);
const text = await Bun.file(lock).text();
const clean = text.replace(
  /(\["[^"]+@[^"]+", )"https?:\/\/[^"]*\/-\/[^"]*\.tgz"/g,
  '$1""',
);
if (clean !== text) await Bun.write(lock, clean);
