import argparse
import asyncio

from dynwinrt import DynWinRTValue, RoApartment, projected_lifetime_scope
from generated.windows.media.playback import (
    MediaPlayer,
    MediaPlayerAudioCategory,
)
from generated.windows.media.speech_synthesis import SpeechSynthesizer
from generated.windows.storage.streams import IRandomAccessStream


async def speak(text: str, smoke: bool) -> None:
    with RoApartment(1), projected_lifetime_scope():
        with SpeechSynthesizer() as synthesizer:
            stream = await synthesizer.synthesize_text_to_stream_async(text)
            if stream is None:
                raise RuntimeError("SpeechSynthesizer returned no stream")

            with stream:
                if smoke:
                    print(
                        "python-tts-ok",
                        {
                            "content_type": stream.content_type,
                            "size": stream.size,
                        },
                    )
                    return

                loop = asyncio.get_running_loop()
                ended = asyncio.Event()

                def on_media_ended(
                    _sender: MediaPlayer | None,
                    _args: DynWinRTValue | None,
                ) -> None:
                    loop.call_soon_threadsafe(ended.set)

                with MediaPlayer() as player:
                    unsubscribe = player.once_media_ended(on_media_ended)
                    try:
                        player.audio_category = MediaPlayerAudioCategory.Speech
                        player.set_stream_source(
                            stream.as_interface(IRandomAccessStream)
                        )
                        player.play()
                        await asyncio.wait_for(ended.wait(), timeout=30)
                    finally:
                        unsubscribe()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--text", default="Hello from dynwinrt.")
    parser.add_argument(
        "--smoke",
        action="store_true",
        help="Synthesize without playing audio.",
    )
    args = parser.parse_args()
    asyncio.run(speak(args.text, args.smoke))


if __name__ == "__main__":
    main()
