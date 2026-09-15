# Job 09: native material mapping and day/night regeneration

Implemented the Unreal polish pass in `scripts/unreal/polish_forum.py` and
`scripts/unreal/polish-forum.sh`.

The architecture actor now restores every component material override to the
FBX static mesh slot's `material_interface` before applying explicit native
assignments. This makes a reimport with changed slot ordering deterministic.
The selective mapping is:

| Imported slot name | Native material | Scope |
| --- | --- | --- |
| `Forum White Sandstone Masonry`, white sandstone masonry, limestone, travertine | `M_ScannedLimestone` | structural masonry only |
| `Forum Geological Strata`, basalt stratum, `Forum Coastal Rock`, weathered/coastal/mossy rock | `M_MossyRock` | geological strata and coastal rock |
| `Forum Architectural Glass`, architectural/conservatory/glazed vault | `M_ArchitecturalGlass` | translucent architectural glazing |
| `Forum Oiled Timber Grain`, oiled structural timber, timber | `M_OriginalWarmTone` | warm structural timber factor |
| bronze, water, dark stone | existing native bronze, water, and basalt materials | legacy explicit categories |

Solar and photovoltaic slots, memory or engraved inlays, and detailed foliage
slots retain the imported material. The obsolete proxy material is only
selected for explicitly named `blockout foliage`, `legacy foliage`, or
`placeholder foliage` slots; the metadata-driven forest and scanned foliage
placement remains unchanged.

`M_ArchitecturalGlass` uses the Unreal `BLEND_Translucent` and
`MSM_DEFAULT_LIT` enums. These names are present as `BLEND_Translucent` and
`MSM_DefaultLit` in UE 5.8 `Engine/Source/Runtime/Engine/Classes/Engine/EngineTypes.h`.
Its base color, zero metallic, low roughness, opacity, and `MP_REFRACTION`
index of 1.45 provide a credible native translucent dielectric without
depending on an unverified shading-model enum. The authored source grain is
carried by the Blender material; the Unreal timber fallback preserves its
warm base factor and avoids importing a broad bark image.

The wrapper accepts `day` or `night` (default `day`) and exports
`FORUM_APPEARANCE` to the Unreal Python process. Day regenerates
`/Game/SolarForum/Maps/ForumCinematic`; night regenerates the existing
`/Game/SolarForum/Maps/ForumNight`. Each mode writes its own
`unreal-environment-{mode}.json` report and log, so rerunning one mode does
not clobber the other. Night also uses a dimmer non-atmospheric directional
light and sky intensity while preserving the same geometry/material pass.

Validation completed without launching Unreal:

```text
python3 -m py_compile scripts/unreal/polish_forum.py   PASS
zsh -n scripts/unreal/polish-forum.sh                  PASS
git diff --check -- scripts/unreal/polish_forum.py scripts/unreal/polish-forum.sh PASS
```

The final material slot names and assignment counts are emitted in the
mode-specific JSON report after root runs the wrapper against the latest
Blender export. This job did not perform an Unreal render, so
`renderVerified` remains false until that run and browser/native review.
