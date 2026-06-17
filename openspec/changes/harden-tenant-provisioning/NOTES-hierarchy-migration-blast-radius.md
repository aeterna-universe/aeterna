# NOTES — Hierarchy Migration Blast Radius (§2.2-B)

**Date:** 2026-04-22  
**Scope:** PR #129 follow-up commits  
**Status:** Historical pre-migration analysis  
**Owner decision:** Option (A) — manifest model wins

---

## Purpose

This note records the historical analysis that led to tenant-scoped hierarchy work.
The original schema had a legacy root wrapper between tenant and organization,
while newer provisioning and runtime flows treated tenant as the real isolation
boundary.

The mismatch created blast radius in four areas:

1. **Schema assumptions** — older hierarchy tables and constraints were built
   around the legacy wrapper layer.
2. **Views** — OPAL-facing hierarchy and permission views needed tenant-first
   columns and ownership in migrations rather than ad hoc runtime setup.
3. **Bootstrap** — bootstrap logic had to ensure canonical tenant rows existed
   before writing hierarchy data.
4. **Apply / render paths** — manifest apply and reverse-render needed one
   consistent tenant-root model.

## Historical conclusion

The correct direction was to:

- treat **tenant** as the in-tenant root,
- move organizations directly under tenant,
- keep migration ownership of canonical views,
- fail loudly on ambiguous legacy data,
- and phase out the legacy wrapper layer rather than preserving it.

## Rollout shape that followed

The work was intentionally split into reviewable slices:

1. documentation + findings correction,
2. migration and bootstrap alignment,
3. hierarchy store and persistence wiring,
4. apply / reverse-render integration.

## Lasting lesson

Hierarchy, authorization, and provisioning all need the same root model.
Any design that keeps a cosmetic wrapper between tenant and organization creates
translation overhead, migration ambiguity, and authorization risk.
