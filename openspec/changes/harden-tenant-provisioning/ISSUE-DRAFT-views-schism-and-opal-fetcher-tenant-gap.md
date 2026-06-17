# Issue Draft — View Ownership Split and OPAL Tenant Filtering Gap

## Summary

This historical issue draft captured two problems that were later fixed:

1. **Canonical hierarchy views had multiple owners** — migrations and runtime
   setup code could both redefine the same OPAL-facing views.
2. **OPAL fetch paths lacked authoritative tenant filtering** — hierarchy,
   user, and agent reads could be assembled from globally merged data instead
   of an explicit tenant-scoped query.

## Why it mattered

Those two problems combined into an authorization risk:

- the wrong view definition could win depending on execution order, and
- downstream policy evaluation could receive cross-tenant entity sets.

## Resolution direction that followed

The workstream converged on five principles:

- canonical OPAL views must be owned by migrations,
- runtime sync code must stop redefining those views,
- fetch handlers must filter by tenant at the SQL layer,
- legacy hierarchy persistence must not override tenant-root reads,
- and the tenant-root hierarchy model must be the only product-facing shape.

## Outcome

That direction later became the basis for relocating view definitions,
threading tenant filters through OPAL fetch handlers, and removing the old
wrapper-root assumptions from active runtime paths.
