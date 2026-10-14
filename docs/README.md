# Leeroy documentation

These Markdown files are the source of the
[Leeroy wiki](https://github.com/hbro/leeroy/wiki). Start at [Home](Home.md).

> [!WARNING]
> The wiki is overwritten from this directory by
> `.github/workflows/wiki.yml` (on every release tag, or when run by hand).
> Manual changes made in the wiki are lost at the next sync: edit the files
> here and open a pull request instead.

- One file per wiki page: the file name is the page title (`-` = space).
  `_Sidebar.md` and `_Footer.md` are the wiki's sidebar and footer.
- Link to other pages as `[Text](Page.md)` (works here); the workflow drops
  the `.md` for the wiki.
- Images live in `screenshots/` (made by `scripts/screenshot.sh readme`) and
  are copied to the wiki along with the pages.
- This `README.md` isn't copied to the wiki.
- Versions: the wiki shows the latest release only. A release's docs are
  this directory at its tag (`/tree/vX.Y.Z/docs`) and in its archive.
