# Findings §2.2 — Hierarchy and reverse-render gaps

This note preserves the high-level conclusion from the original investigation
without carrying forward stale legacy-root terminology.

## Corrected finding

The storage layer for hierarchy already existed, but it was built around an
older pre-tenant-root design. That meant provisioning, reverse-rendering, and
authorization were all reasoning about the same hierarchy through slightly
incompatible shapes.

## What mattered

- The runtime isolation boundary was already **tenant**.
- The hierarchy contract needed to become **tenant -> organization -> team -> project**.
- Reverse-render could only be correct once apply-path storage and read-path
  storage agreed on that same shape.
- Role fan-out and friendly-name rendering depended on the hierarchy cleanup
  landing first.

## Engineering lesson

The main problem was not a missing renderer in isolation. The real issue was a
model split between old hierarchy persistence assumptions and the newer
provisioning contract.

## Resulting direction

The follow-up work therefore focused on:

1. tenant-root hierarchy storage,
2. migration-owned OPAL views,
3. apply-path hierarchy writes,
4. reverse-render from the tenant-root hierarchy,
5. and scope resolution that no longer depended on the old wrapper layer.
