# Replace Translation Visual Fixtures V1

Each fixture contains a deterministic synthetic `source.png`, an `expected_protection.png`, geometry expectations, fixed complete translations, and a cross-platform case manifest.

Run `npm run fixtures:replace-visual` after changing the generator. Run `npm run test:replace-visual` to verify that generated images are current and that geometry, pixel-protection, translation-completeness, and scene-bucket gates behave correctly.

User screenshots are not committed here. A regression discovered from a private screenshot must be represented by a minimal synthetic equivalent or kept in a local-only manifest.
