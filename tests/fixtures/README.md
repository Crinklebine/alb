# Synthetic audio fixtures

tone.flac is an original generated 440 Hz sine wave, 50 ms, mono, 44.1 kHz,
16-bit, with fictional tags. It contains no copyrighted recording.
Created for ALB tests; may be freely used and redistributed.

Regenerate with FFmpeg (not needed to run tests):

```sh
ffmpeg -v error -f lavfi -i "sine=frequency=440:sample_rate=44100:duration=0.05" \
  -ac 1 -c:a flac -metadata title="Fixture Tone" -metadata artist="Test Artist" \
  -metadata album_artist="Test Ensemble" -metadata album="Test Album" \
  -metadata track="3" -metadata disc="2" -y tests/fixtures/tone.flac
```

Encoder versions may produce different bytes. Tests assert semantic metadata,
not an encoder-specific whole-file hash.

## Other supported formats

`tone.m4a`, `tone.mp3`, `tone.ogg`, `tone.wav` are the same original 50 ms
440 Hz, 44.1 kHz generated tone, encoded with FFmpeg using `aac`, `libmp3lame`,
`libvorbis`, and `pcm_s16le` respectively. They use the same fictional tags as
`tone.flac`. The corresponding `untagged.*` files omit the metadata arguments.
They share the unrestricted test-fixture provenance above. No encoder is required
at test runtime. Lossy formats may report encoder-padding duration above 50 ms.

WAV stores title/artist/album/track in RIFF INFO. For the tagged WAV fixture only,
an ID3v2.3 `id3 ` chunk adds `TPE2=Test Ensemble` and `TPOS=2` (Latin-1 text).
This intentionally verifies primary/secondary tag merging without replacing the
RIFF fields. The RIFF file length includes the appended, word-aligned ID3 chunk.
