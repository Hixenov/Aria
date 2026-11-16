#!/usr/bin/env bash
# Generates the audio fixtures in crates/aria-audio/tests/fixtures. Requires ffmpeg (with
# libmp3lame, libvorbis, libopus, and a PNG decoder) on PATH.
set -euo pipefail

out="$(cd "$(dirname "$0")/.." && pwd)/crates/aria-audio/tests/fixtures"
mkdir -p "$out"
cd "$out"

tone="sine=frequency=440:duration=1:sample_rate=44100"
# -bitexact keeps reruns byte-stable (no encoder version or timestamps in the output).
ff() { ffmpeg -hide_banner -loglevel error -y -fflags +bitexact -flags:a +bitexact "$@"; }

ff -f lavfi -i "$tone" -ac 2 -c:a libmp3lame -b:a 128k tone.mp3
ff -f lavfi -i "$tone" -ac 2 -c:a flac tone.flac
ff -f lavfi -i "$tone" -ac 2 -c:a pcm_s16le tone.wav
ff -f lavfi -i "$tone" -ac 2 -c:a aac -b:a 128k tone.m4a
ff -f lavfi -i "$tone" -ac 2 -c:a alac tone-alac.m4a
ff -f lavfi -i "$tone" -ac 2 -c:a libvorbis tone.ogg
ff -f lavfi -i "$tone" -ac 2 -c:a libopus -ar 48000 tone.opus
ff -f lavfi -i "sine=frequency=440:duration=1:sample_rate=48000" -ac 2 -c:a flac tone-48k.flac
ff -f lavfi -i "$tone" -ac 1 -c:a pcm_s16le tone-mono.wav
ff -f lavfi -i "$tone" -ac 2 -c:a pcm_s16be tone.aiff
ff -f lavfi -i "$tone" -ac 2 -c:a pcm_s16le tone.caf

# 16-bit and 24-bit FLAC and WAV made from the same samples (0.1 s of 440 Hz, so each stays
# small): the 24-bit files hold the 16-bit samples shifted up by 8 bits.
short="sine=frequency=440:duration=0.1:sample_rate=44100"
ff -f lavfi -i "$short" -ac 2 -c:a flac depth-16.flac
ff -f lavfi -i "$short" -ac 2 -c:a pcm_s16le depth-16.wav
ff -f lavfi -i "$short" -ac 2 -sample_fmt s32 -c:a flac depth-24.flac
ff -f lavfi -i "$short" -ac 2 -c:a pcm_s24le depth-24.wav

# WAV and AIFF whose data is not integer PCM, for the codec the library stores: float PCM
# (lossless) and A-law, mu-law and IMA ADPCM (lossy). Mono where it keeps a file small.
ff -f lavfi -i "$short" -ac 1 -c:a pcm_f32le float.wav
ff -f lavfi -i "$short" -ac 1 -c:a pcm_f32be float.aiff
ff -f lavfi -i "$short" -ac 2 -c:a pcm_alaw alaw.wav
ff -f lavfi -i "$short" -ac 2 -c:a pcm_mulaw mulaw.aiff
ff -f lavfi -i "$short" -ac 2 -c:a adpcm_ima_wav adpcm.wav

ff -f lavfi -i "$tone" -ac 2 -c:a libmp3lame -b:a 128k \
    -metadata title="Fixture Title" -metadata artist="Fixture Artist" \
    -metadata album="Fixture Album" -metadata track=3 tagged.mp3

# 300x300 generated PNG, embedded as the front cover.
ff -f lavfi -i "color=c=0x3366cc:s=300x300:d=1" -frames:v 1 cover.png
ff -f lavfi -i "$tone" -i cover.png -map 0:a -map 1:v -ac 2 -c:a flac -c:v copy \
    -disposition:v attached_pic cover.flac
rm -f cover.png

# Seeded white noise: unlike a pure tone it has no repeating period, so a seek test can
# tell exactly where in the stream a decoder landed. Lossy formats only.
noise="anoisesrc=color=white:amplitude=0.3:duration=1:sample_rate=44100:seed=7"
ff -f lavfi -i "$noise" -ac 2 -c:a libmp3lame -b:a 128k noise.mp3
ff -f lavfi -i "$noise" -ac 2 -c:a aac -b:a 128k noise.m4a
ff -f lavfi -i "$noise" -ac 2 -c:a libvorbis noise.ogg
ff -f lavfi -i "$noise" -ac 2 -c:a libopus -ar 48000 noise.opus

# Two consecutive halves of one continuous 440 Hz tone, cut at a sample boundary (22050 = 0.5 s).
ff -f lavfi -i "sine=frequency=440:duration=1:sample_rate=44100" -ac 2 -c:a flac whole.flac
ff -i whole.flac -af "atrim=end_sample=22050,asetpts=N/SR/TB" -c:a flac silence-gap-a.flac
ff -i whole.flac -af "atrim=start_sample=22050,asetpts=N/SR/TB" -c:a flac silence-gap-b.flac
rm -f whole.flac
