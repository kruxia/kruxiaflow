# Kruxia Flow Licensing FAQ

## Overview

Kruxia Flow is licensed under the **GNU Affero General Public License v3 (AGPLv3)**, an OSI-approved open source license. We also offer commercial licenses for organizations that prefer not to comply with the AGPL's requirements.

This FAQ explains our licensing philosophy, clarifies common questions about AGPL compliance, and helps you determine which license is right for your use case.

---

## Why We Chose AGPL

### Our Philosophy

We believe in building sustainable open source software. The AGPL allows us to:

1. **Keep Kruxia Flow fully open source** — The AGPL is approved by the Open Source Initiative (OSI) and the Free Software Foundation (FSF). Kruxia Flow is not "source available" — it's genuinely open source with all the freedoms that entails.

2. **Ensure improvements benefit everyone** — If someone modifies Kruxia Flow itself and offers it as a service, those improvements should flow back to the community. This is the core principle of copyleft.

3. **Build a sustainable business** — Revenue from commercial licenses helps fund continued development, ensuring Kruxia Flow remains actively maintained and improved.

4. **Prevent exploitation without contribution** — The AGPL ensures that companies building competing workflow orchestration services based on Kruxia Flow contribute their improvements back, while still allowing broad use in applications.

### What We're NOT Trying to Do

We are **not** trying to:

- Force you to open source your applications that use Kruxia Flow
- Create legal uncertainty to pressure commercial license sales
- Restrict legitimate use of Kruxia Flow in proprietary software
- Interpret the AGPL more broadly than its text supports

We want Kruxia Flow to be widely adopted. The AGPL exists to protect the project, not to create barriers for users.

---

## Common Questions

### Can I use Kruxia Flow in my proprietary application?

**Yes.** If your application communicates with Kruxia Flow through its APIs (HTTP REST, WebSocket, gRPC), your application is **not** a derivative work and does not need to be licensed under the AGPL.

Kruxia Flow is a service that your application talks to — like a database. Just as using PostgreSQL doesn't make your application GPL-licensed, using Kruxia Flow doesn't make your application AGPL-licensed.

### Does using the Kruxia Flow client libraries require me to open source my code?

**No.** Our official client libraries and SDKs are licensed under the **MIT License**, not AGPL. You can use them in proprietary applications without any copyleft obligations.

This is an intentional design choice. We want integration with Kruxia Flow to be frictionless.

### What if I deploy Kruxia Flow as part of my infrastructure?

**That's fine.** Running unmodified Kruxia Flow as part of your internal or customer-facing infrastructure does not trigger any source code disclosure requirements.

The AGPL's network clause (Section 13) only applies when you **modify** Kruxia Flow and make it available over a network. Using Kruxia Flow as-is — even in a commercial SaaS product — requires nothing beyond what any AGPL software requires: preserving copyright notices and providing access to Kruxia Flow's source code (which is already publicly available).

### What counts as "modifying" Kruxia Flow?

Modification means changing Kruxia Flow's own source code. This includes:

- Forking the repository and changing the code
- Patching the Kruxia Flow binary
- Adding features directly to Kruxia Flow's codebase

Modification does **NOT** include:

- Calling Kruxia Flow's APIs from your application
- Writing workflows that run on Kruxia Flow
- Configuring Kruxia Flow through its documented configuration options
- Building plugins or extensions that communicate with Kruxia Flow through defined interfaces
- Deploying Kruxia Flow alongside other services in your infrastructure

### What if I modify Kruxia Flow for internal use only?

**No disclosure required.** The AGPL (like the GPL) only requires source code disclosure when you convey the software to others or make it available over a network to external users. Internal use — even with modifications — has no disclosure requirement.

### What if I modify Kruxia Flow and offer it as a service to customers?

In this case, you must make the source code of your modified version available to users who interact with it over the network. This is the core purpose of the AGPL — ensuring that service providers contribute improvements back to the community.

Alternatively, you can purchase a commercial license that removes this requirement.

### Can I run Kruxia Flow in a microservice architecture without open sourcing my other services?

**Yes, absolutely.** Your other services communicate with Kruxia Flow over network protocols (HTTP, WebSocket, gRPC). They are separate programs, not derivative works.

The AGPL does not propagate across network boundaries. A microservice that calls Kruxia Flow's API is no different from a web application that uses an AGPL-licensed database — the calling application is not affected by the AGPL.

### Does embedding Kruxia Flow or linking to it make my application AGPL?

If you're using Kruxia Flow as intended — as a standalone service that your application communicates with — then **no**.

If you're doing something unusual like embedding Kruxia Flow's Rust code directly into your application binary, that would create a combined work subject to the AGPL. But this isn't a typical use case. Kruxia Flow is designed to run as a separate service.

### What about plugins or workflow definitions?

**Plugins**: If your plugin communicates with Kruxia Flow through its defined plugin API, it is not a derivative work. You can license your plugins however you choose.

**Workflow definitions**: Workflow definitions (YAML, JSON, or code that describes your workflows) are your content, not modifications to Kruxia Flow. They're analogous to SQL queries you run against a database — they don't become GPL-licensed just because the database is GPL-licensed.

### Do I need to display AGPL notices in my application?

Only if you distribute Kruxia Flow itself or a modified version of it. If you're just using Kruxia Flow as a service (the normal case), you don't need to include AGPL notices in your own application.

---

## Commercial Licensing

### When should I consider a commercial license?

A commercial license is appropriate if:

1. **You want to modify Kruxia Flow** and offer it as a service without sharing your modifications
2. **Your organization has a blanket policy** against AGPL software (some enterprises do)
3. **You want contractual support guarantees**, SLAs, or indemnification
4. **You prefer the simplicity** of a commercial agreement over open source license compliance

### What does the commercial license include?

A Kruxia Flow commercial license provides:

- **No copyleft obligations** — Modify and deploy without source disclosure requirements
- **Enterprise support** — Direct access to our engineering team
- **SLAs** — Guaranteed response times and uptime commitments
- **Indemnification** — Legal protection for your use of Kruxia Flow
- **Priority features** — Input into our roadmap and priority bug fixes

Contact us at **licensing@kruxia.com** to discuss commercial licensing.

---

## Summary: Do I Need to Open Source My Code?

| Scenario                                          | Open Source Required?                    |
|---------------------------------------------------|------------------------------------------|
| Using Kruxia Flow as a service via its APIs       | **No**                                   |
| Using our MIT-licensed client libraries            | **No**                                   |
| Running unmodified Kruxia Flow in production       | **No**                                   |
| Writing workflows that run on Kruxia Flow          | **No**                                   |
| Building plugins that use Kruxia Flow's plugin API | **No**                                   |
| Modifying Kruxia Flow for internal use only        | **No**                                   |
| Modifying Kruxia Flow and distributing it          | **Yes** (or get commercial license)      |
| Modifying Kruxia Flow and offering it as a service | **Yes** (or get commercial license)      |

---

## Our Commitment

We commit to interpreting the AGPL in good faith and in accordance with its text. We will not pursue aggressive or novel legal theories to expand the scope of the AGPL beyond what it plainly requires.

If you have questions about whether your use case requires a commercial license, please contact us at **licensing@kruxia.com**. We're happy to provide guidance — our goal is to make Kruxia Flow easy to adopt, not to create legal uncertainty.

---

## Additional Resources

- [Full AGPL v3 License Text](https://www.gnu.org/licenses/agpl-3.0.html)
- [FSF's GPL FAQ](https://www.gnu.org/licenses/gpl-faq.html) (much of which applies to AGPL)
- [Kruxia Flow Documentation](https://kruxiaflow.com/docs)
- [Commercial Licensing Inquiries](mailto:licensing@kruxia.com)

---

*Last updated: February 2026*
