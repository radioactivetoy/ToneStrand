# Built-in effects lists

One folder per HX Edit version (for example `3.82/`). Each holds:

- `catalog.json.gz`: the effect names, knob names, ranges and categories converted from HX Edit's resource files.
- `meta.json`: the HX Edit version, the date it was made, and the name and size of each HX Edit file it was made from.

This is Line 6 material, kept by the owner's decision under the notice in the main README ("Third-party material and
trademarks"). Nothing else from HX Edit goes in this folder.

## Add a version

Install the new HX Edit, then run:

```sh
tonestrand --export-catalog "C:\Program Files (x86)\Line6\HX Edit\res" catalogs 3.83
```

It writes `catalogs/3.83/` and refuses if that folder already exists. Commit the two files.

The app builds in the newest version. If HX Edit is installed and newer, the app reads HX Edit's own files instead.
