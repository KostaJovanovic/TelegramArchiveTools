# Adding a model for Summaries

Summaries are written by a model running on this computer, under llama.cpp's
`llama-server`. Both live in one **models folder**: `Models\` beside
`TelegramTools.exe` unless you choose another in **Settings › Summaries ›
Models folder**. Everything below goes into that folder, either directly or in
a folder of its own inside it. The app looks one level down, so unzipping into
a subfolder is fine.

```
Models\
  llama-b6000-bin-win-cuda-12.4-x64\   ← llama.cpp, unzipped
    llama-server.exe  ggml-cuda.dll  cudart64_12.dll  …
  gemma-4-26B-A4B-it-UD-Q4_K_XL.gguf   ← the model
  paddleocr-vl\                         ← optional: text in photos
    PaddleOCR-VL-1.5.gguf  PaddleOCR-VL-1.5-mmproj.gguf  chat_template.jinja
```

## 1. llama.cpp (once)

Open the [llama.cpp releases](https://github.com/ggml-org/llama.cpp/releases)
and, under the newest release's **Assets**, take:

- **NVIDIA card:** `llama-…-bin-win-cuda-<version>-x64.zip` **and**
  `cudart-llama-bin-win-cuda-<version>-x64.zip`, with the same CUDA version
  in both names. The second holds the CUDA runtime the first needs.
- **AMD or Intel card:** `llama-…-bin-win-vulkan-x64.zip`.

Unzip into the same folder inside the models folder. **Settings › Summaries ›
Model server** names the `llama-server.exe` it found. A newer release later is
the same steps: unzip it, and delete the old folder.

## 2. A model, from Hugging Face

1. On [Hugging Face](https://huggingface.co/models?library=gguf), search for
   the model's name and **GGUF**, e.g. `gemma 4 26B A4B GGUF`. llama.cpp reads
   only `.gguf` files. Repositories by `unsloth`, `bartowski` and `ggml-org`
   usually have every size.
2. Open the repository's **Files and versions** tab. Each `.gguf` is the same
   model at a different *quantisation*: smaller files are faster and lose a
   little quality.
3. **Pick by your graphics card's memory.** The file should be a few GB
   smaller than the card's memory, because the conversation being read (the
   context) needs room too. `Q4_K_M`, or Unsloth's `UD-Q4_K_XL`, is the usual
   choice. A file bigger than the card still runs, partly from RAM, several
   times slower.
4. Download it with the ↓ beside its name. Skip the `mmproj…` files: they let
   a model look at pictures, which a summary does not use.
5. Some models ask you to sign in and **accept a licence** on their page
   before the download starts.
6. A large model is sometimes split: `…-00001-of-00003.gguf`, `…-00002-…`.
   Download every piece into the same folder. The app lists only the first
   piece, and llama.cpp finds the rest.

Put the file in the models folder. **Settings › Summaries › Model** lists every
model there; choose it. With none chosen, the first Gemma is used.

**Which model.** The prompts were written and tested with **Gemma 4 26B-A4B**,
which fits a 16 GB card at `UD-Q4_K_XL`. Other instruction-tuned models work,
but may follow the rules about citations and positions less well. The
default **Server options** (`-c 32768 -ngl 99 …`) are for that model on that
card. With a smaller card, lower `-ngl` to keep some layers in RAM, or `-c` to
shrink the context.

## 3. Text in photos (optional)

Posters and screenshots are read by **PaddleOCR-VL** before the summary is
written. Search Hugging Face for `PaddleOCR-VL GGUF`. Put three files in a
folder of their own inside the models folder, e.g. `paddleocr-vl\`:

- the model (`PaddleOCR-VL-….gguf`);
- its `mmproj` file, which is needed here, since this model reads pictures;
- `chat_template.jinja`, if the repository has one.

Its own folder matters: there, a projector with any name counts as the OCR's.
Turn the reading off with **Settings › Summaries › Read text in photos**.

## If it does not start

- **"There is no llama-server and model to start":** the models folder has no
  `llama-server.exe` or no `.gguf`. Check the two Settings lines that name
  what was found.
- **It stops while loading:** `llama-server.log` beside `tgx.log`, in
  `TelegramToolsData\`, says why. Usually it ran out of memory: choose a
  smaller file, or lower `-ngl` or `-c`.
- **Another program has a model on port 8080:** the Summaries view says which
  and offers **Stop it**. You can also change **Port**.
