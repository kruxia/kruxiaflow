# Kruxia Flow Licensing FAQ

## Overview

Kruxia Flow — the engine and the client SDKs alike — is licensed under the
**Apache License, Version 2.0**, an OSI-approved permissive open source license.
(Rust crates published to crates.io are dual-licensed **MIT OR Apache-2.0**,
following the Rust ecosystem convention.) There is no copyleft, no
source-disclosure requirement, and no dual-license gate: you can use, modify,
embed, and redistribute Kruxia Flow in commercial and proprietary systems.

---

## Why Apache 2.0

1. **Zero adoption friction.** You should never need a legal review to run a
   workflow engine. Apache 2.0 is one of the most widely approved licenses in
   enterprise policy lists.

2. **An explicit patent grant.** Apache 2.0 grants every user a patent license
   from every contributor and terminates it for anyone who initiates patent
   litigation over the project. MIT is silent on patents; Apache 2.0 is not.

3. **Genuinely open source.** Apache 2.0 is approved by the Open Source
   Initiative and the Free Software Foundation. Kruxia Flow is not
   "source-available" — it is open source with all the freedoms that entails.

4. **A sustainable business without a license gate.** Kruxia plans to offer
   commercial services around Kruxia Flow — hosted/managed offerings, support,
   and SLAs — rather than selling exceptions to a copyleft license.

### A note on history

Kruxia Flow releases prior to July 2026 were licensed under the GNU Affero
General Public License v3.0 (AGPL-3.0). In July 2026 the project was relicensed
to Apache 2.0 to remove adoption friction. Old releases remain available under
their original license; all current and future releases are Apache 2.0.

---

## Common Questions

### Can I use Kruxia Flow in my proprietary application?

**Yes.** No conditions beyond the standard Apache 2.0 requirements (preserve
copyright/NOTICE attributions when you redistribute the software itself).

### Can I modify Kruxia Flow and keep my changes private?

**Yes.** Apache 2.0 has no copyleft. You may modify Kruxia Flow and deploy it —
internally or as a customer-facing service — without disclosing your changes.
We'd love to receive improvements as pull requests, but that's an invitation,
not an obligation.

### Can I offer Kruxia Flow as a hosted service?

**Yes.** Apache 2.0 permits it. Note that the license does not grant rights to
the **Kruxia** or **Kruxia Flow** trademarks (see below) — a hosted offering
must not present itself as an official Kruxia service.

### Can I embed Kruxia Flow's Rust crates directly in my application?

**Yes.** Linking, embedding, and static compilation are all fine. Retain the
LICENSE and NOTICE files when you redistribute.

### What about the client SDKs?

The official client libraries and SDKs (e.g.,
[kruxiaflow-python](https://github.com/kruxia/kruxiaflow-python)) are licensed
under **Apache-2.0**, same as the engine (SDK versions released before July 2026
were MIT). Rust crates on crates.io are dual-licensed **MIT OR Apache-2.0** per
Rust ecosystem convention — pick whichever suits your project. Use them all in
proprietary applications freely.

### What do I have to do if I redistribute Kruxia Flow?

The standard Apache 2.0 conditions: include a copy of the license, retain
copyright/patent/trademark/attribution notices and the NOTICE file, and state
significant changes you made to the files you modified. Nothing else.

### Do workflow definitions have any license implications?

**No.** Workflow definitions (YAML, JSON, or SDK code describing your
workflows) are your content — analogous to SQL queries run against a database.

### Trademarks

The Apache 2.0 license does not grant permission to use the Kruxia or
Kruxia Flow names and logos, except as required for reasonable and customary
use in describing the origin of the software. If you fork or offer a hosted
version, name it in a way that doesn't imply it is an official Kruxia offering.

---

## Commercial Services

You do not need to buy anything to use Kruxia Flow, at any scale, for any
purpose. Kruxia offers optional commercial services for organizations that
want them:

- **Support agreements** — direct access to the engineering team, guaranteed
  response times
- **Managed hosting** — planned; contact us for early interest
- **Consulting** — workflow design, migration, and cost-governance reviews

Contact **licensing@kruxia.com**.

---

## Summary

| Scenario                                            | Allowed? | Conditions                                  |
|-----------------------------------------------------|----------|----------------------------------------------|
| Use Kruxia Flow in a proprietary application        | **Yes**  | None                                         |
| Run unmodified Kruxia Flow in production/SaaS       | **Yes**  | None                                         |
| Modify Kruxia Flow and keep changes private         | **Yes**  | None                                         |
| Embed/link the Rust crates in your binary           | **Yes**  | Retain LICENSE + NOTICE on redistribution    |
| Redistribute Kruxia Flow (modified or not)          | **Yes**  | Standard Apache 2.0 attribution conditions   |
| Offer a hosted Kruxia Flow service                  | **Yes**  | Don't use Kruxia trademarks as your own      |
| Use the client SDKs anywhere                        | **Yes**  | Standard Apache 2.0 attribution conditions   |

---

## Additional Resources

- [Full Apache License 2.0 Text](https://www.apache.org/licenses/LICENSE-2.0)
- [Kruxia Flow LICENSE file](../LICENSE) and [NOTICE file](../NOTICE)
- [Kruxia Flow Documentation](https://kruxiaflow.com/docs)
- [Commercial Services Inquiries](mailto:licensing@kruxia.com)

---

*Last updated: July 2026*
