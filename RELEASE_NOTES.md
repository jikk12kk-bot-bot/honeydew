Honeydew 0.0.8: choose Android's audio driver.

- Settings (Android). New `Audio driver:` under `Sound mode:` picks which Android audio system plays the sound:
  - `Per sound mode` (the default), as FL Studio Mobile: `NATIVE` through AAudio (Oboe), `FAST` and `SAFE` through OpenSL ES, `ULTRASAFE` through AudioTrack.
  - `AAudio`: every mode through AAudio, Android's newest and lowest-latency audio (OpenSL ES on Android 8.0). This is how 0.0.6 and 0.0.7 played.
  - `OpenSL ES`: every mode through Android's older native audio.
  - `AudioTrack`: every mode through Android's oldest audio, the highest latency and the widest compatibility.
  The sound mode still sets the buffer size. If a driver cannot open on a phone, the app falls back to the next one (AudioTrack to Oboe, Oboe to AAudio).
