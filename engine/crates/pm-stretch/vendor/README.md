# Vendored Signalsmith Stretch

Pitch-preserving time stretch for pm-audio (plan section 16: "pitch preserved by
default (Signalsmith Stretch, our own binding)"). Both libraries are MIT-licensed
(section 24 item 7: MIT is on the `cargo-deny` allow list); their license files sit
next to the sources and their notices must ship with the app.

| Directory | Upstream | Version | Commit |
|---|---|---|---|
| `signalsmith-stretch/` | https://github.com/Signalsmith-Audio/signalsmith-stretch | 1.4.0 (header reports 1.3.2) | a670068d9aeb64913331d5cc29337b19a457a7df |
| `signalsmith-linear/` | https://github.com/Signalsmith-Audio/linear | 0.6.4 | de55e6a50ffcf6f8f43f649692d94691c7025151 |

Only the headers the stretch includes are vendored, unmodified:
`signalsmith-stretch/signalsmith-stretch.h` (its upstream root header) and
`signalsmith-linear/{fft.h,stft.h}`. No platform FFT backend is enabled, so output
is bit-identical across machines.

SHA-256:

```
1188667959ac19dd40c0a6abbce694e44705615ec4f0e8db8af0f1cfb4c5dea7  signalsmith-stretch/signalsmith-stretch.h
ee2ef82481ffb445ecdd4b3a4c1f82c0cddb2da8fe39d8e8dc384fffb3e7f06f  signalsmith-stretch/LICENSE.txt
fb55f69ac14cdbac66b9c76053069ff63bc36d18af20d8c028b9acc0aad6516a  signalsmith-linear/fft.h
7422ff5ddc2a017dc0587dcce99ce5ba30a2414bc2996081f6b00d56905d9b8c  signalsmith-linear/stft.h
072b5e9eb5b22880bdf6256324654cf3ce53dba828edb911921121e1be09d9c3  signalsmith-linear/LICENSE.txt
```

To update: copy the same files from the new tags, update this table and the hashes,
and rerun `cargo test -p pm-stretch` and the pm-audio stretch tests
(`docs/engine/pm-audio.md`, "Pitch-preserving stretch").
