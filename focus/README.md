# Built-in Focus mode tones

One folder per Helix Stadium Native version (for example `1.4.1/`). Each holds:

- `focus.json.gz`: for each effect, five named tones (a centre and four corners) and the value each parameter takes in them, already
  matched to the pedal's effect symbols and parameter positions (made with HX Edit's effects list). Focus mode blends them as you move on a plane.
- `meta.json`: the Helix Stadium Native version, the date, which effects list the positions came from, and how many models were converted.

This is Line 6 material (the tones and their names come from Helix Stadium Native's `focus-view-xy-defaults`), kept by the owner's decision under
the notice in the main README ("Third-party material and trademarks"). Nothing else from Helix Stadium Native goes in this folder. The Helix
Stadium-only amps (the `Agoura` ones) and models newer than the effects list are left out.

## Add a version

Install the new Helix Stadium Native, then run:

```sh
tonestrand --export-focus "C:\ProgramData\Line 6\Helix Stadium Native\res\meta-data\focus-view-xy-defaults" focus 1.5.0
```

It writes `focus/1.5.0/` and refuses if that folder already exists. Commit the two files. The app builds in the newest version.
The pack needs the effects list it was made with: when a newer HX Edit adds models, make the pack again to give them a pad.
