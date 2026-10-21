"""Pinned PDF2zh 2.9 / BabelDOC bridge. Credentials never enter this process.

JSON lines on inherited pipes carry model messages and progress. Kivio owns model
protocol/authentication, cancellation and the lifetime of this whole process group.
"""
import asyncio
import json
import logging
import sys
import threading
from pathlib import Path
from types import SimpleNamespace

wire = sys.stdout
sys.stdout = sys.stderr  # Third-party diagnostics must not corrupt the protocol.
logging.basicConfig(level=logging.ERROR)
write_lock = threading.Lock()
request_lock = threading.Lock()


def emit(event):
    with write_lock:
        wire.write(json.dumps(event, ensure_ascii=False) + "\n")
        wire.flush()


def completion(**request):
    # PDF2zh's worker threads share a single pipe: a reply belongs to one request.
    with request_lock:
        emit({"type": "model", "messages": request["messages"]})
        line = sys.stdin.readline()
        if not line:
            raise RuntimeError("Kivio translation bridge closed")
        reply = json.loads(line)
        if "error" in reply:
            raise RuntimeError(reply["error"])
        return SimpleNamespace(choices=[SimpleNamespace(message=SimpleNamespace(content=reply["text"]))])


async def main():
    from pdf2zh_next.config.model import SettingsModel
    from pdf2zh_next import high_level
    from pdf2zh_next.translator.translator_impl.openai import OpenAITranslator
    from pdf2zh_next.translator.rate_limiter.qps_rate_limiter import QPSRateLimiter
    from babeldoc.format.pdf.high_level import async_translate

    request = json.loads(sys.stdin.readline())
    if request.get("pages") is not None:
        import pymupdf
        with pymupdf.open(request["input"]) as source:
            if not 1 <= int(request["pages"]) <= len(source):
                raise ValueError("Requested page is outside the document")
    settings = SettingsModel.model_validate({
        "translation": {"lang_in": request["sourceLang"], "lang_out": request["targetLang"],
                        "output": request["output"], "qps": 1, "pool_max_workers": 1,
                        "no_auto_extract_glossary": True},
        "pdf": {"pages": request.get("pages"), "translate_table_text": False,
                "ocr_workaround": False, "auto_enable_ocr_workaround": False,
                "watermark_output_mode": "no_watermark"},
        "translate_engine_settings": {"translate_engine_type": "OpenAI",
                                      "openai_model": request["model"],
                                      "openai_api_key": "kivio-pipe-no-credential"},
    })
    settings.validate_settings()
    translator = OpenAITranslator(settings, QPSRateLimiter(1))
    # Retain PDF2zh prompts, formula placeholders and its translation cache.
    translator.add_cache_impact_parameters("kivio_provider", request["providerId"])
    translator.client.close()
    translator.client = SimpleNamespace(chat=SimpleNamespace(completions=SimpleNamespace(create=completion)))
    # The pinned factory has no translator injection argument and otherwise runs
    # its own network health check before returning. Supply our already bridged
    # translator before that check; this process never makes provider HTTP calls.
    high_level.get_translator = lambda _: translator
    high_level.get_term_translator = lambda _: None
    config = high_level.create_babeldoc_config(settings, Path(request["input"]))
    async for event in async_translate(config):
        kind = event["type"]
        if kind == "finish":
            result = event["translate_result"]
            emit({"type": "finish", "mono": str(result.mono_pdf_path), "dual": str(result.dual_pdf_path)})
            return
        if kind == "error":
            raise RuntimeError(str(event.get("error", "PDF translation failed")))
        if "overall_progress" in event:
            emit({"type": "progress", "percent": event["overall_progress"], "stage": event.get("stage", "")})
    raise RuntimeError("PDF2zh did not return an output")


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except Exception as error:
        emit({"type": "error", "error": str(error)[:2000]})
        sys.exit(1)
