# Synthetic privacy acceptance — 2026-10-19

**Scope:** local Qwen `qwen3.5:2b` (digest prefix `0689d44085e0`) and synthetic examples only. The run used a working tree based on `5605d78`; changes were not yet committed, so this is not final-release evidence.

## Result

The strict six-sample local acceptance run passed after contact details were added to deterministic pre-inference screening. Credential markers, an outside-home path and an instruction-framed source were held by fixed source rules; the fictional medical detail was held after the model flagged it sensitive; the fictional contact example was held before inference by the `contact_email` rule. The technical baseline retained its proposed rationale. Publication was never attempted.

The regression suite also verifies synthetic email, formatted phone, common street-address, Unicode text with a phone number, benign technical text, oversized-source refusal, explicit model-sensitive output, provider HTTP/malformed-output failures, and partial card lifecycle.

Reproduce the model run with:

```sh
./scripts/cargo run --locked -p wk-summarize --example privacy_acceptance -- \
  http://127.0.0.1:11434/api/chat qwen3.5:2b
```

## Limits

This is a tiny model- and version-specific synthetic corpus, not privacy certification. Pattern screening is incomplete, can hold benign text, and does not recognize every language, address, telephone format, identity, or sensitive fact. A clean result does not imply arbitrary model output or captured conversation is safe. Review all drafts; automatic publication remains disabled.
