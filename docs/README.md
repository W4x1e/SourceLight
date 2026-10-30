# Documentation

Everything written down about SourceLight, and the order to read it in.

## Learning how it works

Read these in order the first time.

1. **[../README.md](../README.md)** — what the app does, for the people who use it.
2. **[ARCHITECTURE.md](ARCHITECTURE.md)** — the system in diagrams: what runs
   where, where data lives, and what happens when you add a folder, drag a
   slider, or turn on a vision model.
3. **[CODEBASE-GUIDE.md](CODEBASE-GUIDE.md)** — a walk through every part of the
   code, teaching the Rust and React it uses along the way.
4. **[../CONTRIBUTING.md](../CONTRIBUTING.md)** — pull requests and the
   contributor licence agreement, reporting raw files that decode wrongly,
   building on Windows and on a Mac, the check gate, dependency licences,
   commit messages.

## By subject

| Document | What it covers |
|---|---|
| [MACOS.md](MACOS.md) | Where the Mac port stands: what works, what was measured, what is left, and the decisions that need an owner |
| [MOBILE.md](MOBILE.md) | The phone app: what it does, how it links to the desktop, editing on the phone, and how to build it |
| [WINDOWS.md](WINDOWS.md) | Making it feel like a Windows app, the counterpart of the Mac plan: what is done, what is open, and what needs a look in the running app |
| [AI-MODEL-SPIKES.md](AI-MODEL-SPIKES.md) | Why the AI models are the ones they are: SigLIP 2 over CLIP, Qwen3-VL over Florence-2, how people are grouped |
| [AI-EDIT.md](AI-EDIT.md) | Letting an AI model edit the photo open in Develop: the editing server, connecting a client, and what is not done yet |
| [I18N.md](I18N.md) | The interface's five languages: where the words live, the rules for adding text, and the checks |
| [LIBRAW.md](LIBRAW.md) | The vendored raw decoder: what is recorded about it, how a release is noticed, how to roll forward |
| [BENCHMARKS.md](BENCHMARKS.md) | What the catalogue costs at 20,000 and 50,000 photos, and the fixes that measurement drove |
| [UPDATES.md](UPDATES.md) | The self-updater: off until configured, how to switch it on, releasing with it |
| [LICENSING.md](LICENSING.md) | The MPL-2.0 and what it means in practice; the third-party, notice and AI-model policy; what redistributors owe |
| [../PRIVACY.md](../PRIVACY.md) | What stays on the computer, and every network request the desktop and phone apps can make |
| [../PRODUCT.md](../PRODUCT.md) | Who it is for, what it promises, and its principles |
| [../DESIGN.md](../DESIGN.md) | The design system: colour, type, layout — authoritative for the interface |
| [../CHANGELOG.md](../CHANGELOG.md) | Every user-visible change, for release notes |
| [type-baseline/](type-baseline/audit.md) | Screenshots and a text-clipping audit of every surface, the baseline for type changes |
| [../scripts/model-eval/README.md](../scripts/model-eval/README.md) | The harness behind the AI model spikes |

## Keeping them true

A document that disagrees with the code is worse than none. When a change makes
one of these wrong, fix it in the same pull request. The code comments are the
most detailed layer: most modules open with a paragraph on *why* they are
built the way they are.
