# Russian speech fixture

`voice-ru.pcm`: 16 kHz, signed 16-bit little-endian mono PCM. Generated locally with the installed Windows SAPI Microsoft Irina voice: «Проверка голосового ввода. Один, два, три.» No microphone or personal recording was used. The fixture is embedded only in tests, not in the app.

The native pipeline regression feeds this speech with leading silence, short pauses and a four-second pause to the real bundled Vosk and Earshot VAD. It verifies logical record boundaries rather than requiring an exact transcription string.
