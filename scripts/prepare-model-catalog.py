"""Refresh pinned model metadata from the export authors; never downloads weights.

Run manually when adding/upgrading a curated model. Review the resulting diff.
The app uses the committed JSON offline, without querying a model hub at startup.
"""
import hashlib
import json
from pathlib import Path
from urllib.request import urlopen

MODELS = [
    dict(id="gigaam-v3-e2e", name="GigaAM v3 E2E", engine="gigaam",
         repo="csukuangfj/sherpa-onnx-nemo-transducer-punct-giga-am-v3-russian-2025-12-16",
         description="Русская речь с пунктуацией и заглавными буквами. Текст появляется после завершения фразы. Кодировщик INT8 для CPU.",
         license="MIT", source="https://huggingface.co/ai-sage/GigaAM-v3",
         files=["encoder.int8.onnx", "decoder.onnx", "joiner.onnx", "tokens.txt", "LICENSE"]),
    dict(id="nemotron-3-5-ru", name="Nemotron 3.5 ASR", engine="nemotron",
         repo="csukuangfj2/sherpa-onnx-nemotron-3.5-asr-streaming-0.6b-560ms-int8-2026-06-11",
         description="Потоковая диктовка на русском с пунктуацией. Предварительный текст появляется во время речи. Вариант INT8 для CPU.",
         license="OpenMDW-1.1", source="https://huggingface.co/nvidia/nemotron-3.5-asr-streaming-0.6b",
         files=["encoder.int8.onnx", "decoder.int8.onnx", "joiner.int8.onnx", "tokens.txt"]),
]

def get(url):
    with urlopen(url, timeout=60) as response:
        return response.read()

def main():
    entries = []
    for definition in MODELS:
        definition = definition.copy()
        repo = definition.pop("repo")
        revision = json.loads(get(f"https://huggingface.co/api/models/{repo}"))["sha"]
        tree = json.loads(get(f"https://huggingface.co/api/models/{repo}/tree/{revision}"))
        remote = {item["path"]: item for item in tree}
        files = []
        for name in definition.pop("files"):
            item = remote[name]
            url = f"https://huggingface.co/{repo}/resolve/{revision}/{name}"
            digest = item.get("lfs", {}).get("oid")
            if not digest:
                digest = hashlib.sha256(get(url)).hexdigest()
            files.append(dict(name=name, url=url, size=item["size"], sha256=digest))
        if definition["engine"] == "nemotron" and "LICENSE" in remote:
            item = remote["LICENSE"]
            url = f"https://huggingface.co/{repo}/resolve/{revision}/LICENSE"
            files.append(dict(name="LICENSE", url=url, size=item["size"], sha256=hashlib.sha256(get(url)).hexdigest()))
        entries.append(dict(**definition, revision=revision, files=files))
    target = Path(__file__).resolve().parents[1] / "src-tauri" / "model-catalog.json"
    target.write_text(json.dumps(entries, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    for entry in entries:
        print(entry["id"], entry["revision"], sum(item["size"] for item in entry["files"]))

if __name__ == "__main__":
    main()
