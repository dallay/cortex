# Accessibility

Accessibility ensures that everyone, regardless of ability or assistive technology, can perceive, understand, navigate, and interact with our products. In a project that bridges applications and AI providers, inclusive access
is not optional — it is a baseline quality requirement. This document describes our accessibility commitments, what we expect from contributors, and how to report barriers.

## Priorities

We aim to make the Rook dashboard and all user-facing interfaces usable by the broadest possible audience. Our accessibility target is WCAG 2.2 Level AA as an aspirational goal, not a verified conformance claim. We have not
yet completed a formal conformance evaluation.

We prioritize these areas:

• Keyboard navigation — every interactive element is reachable and operable without a mouse.
• Screen reader compatibility — content structure, labels, and live regions convey meaning to assistive technology users.
• Visual clarity — sufficient color contrast, readable typography, and no reliance on color alone to convey information.
• Content and language — plain language in documentation, interface text, and error messages.

We address accessibility issues alongside functional bugs and features, not as an afterthought.

## Contributor expectations

Contributors are expected to maintain or improve accessibility with every change. When submitting user-facing changes:

• Test keyboard navigation — tab through every new or modified interactive element and verify focus order, visible focus indicators, and operability.
• Check with a screen reader — use VoiceOver (macOS), NVDA (Windows), or TalkBack (Android) to verify that labels, roles, and announcements are correct.
• Run automated checks — the project uses Biome, which includes accessibility linting rules (lint/a11y). Ensure biome ci . passes without accessibility errors before submitting.
• Provide evidence — for significant interface changes, include a brief note in the PR description describing what was tested and any accessibility considerations.
• Do not reduce accessibility — do not remove labels, hide focus indicators, lower contrast, or break semantic structure to achieve a visual design.

These are minimum expectations. Going beyond them is encouraged.

## Reporting accessibility issues

Report accessibility barriers through [GitHub Issues](https://github.com/dallay/cortex/issues/new). Use the same issue tracker as bugs and features — accessibility barriers are bugs.

When reporting, include as much of the following context as is practical:

• The task you were trying to complete — what were you trying to do?
• The affected page or component — URL or component name.
• What you observed — what happened instead of the expected behavior?
• Your environment — browser, operating system, and assistive technology (if applicable).
• Screenshots or recordings — optional and helpful, but never required.

You do not need to disclose a disability to report an accessibility issue. If you need help using the product, you can ask in the issue or in discussions.

### Severity

We classify accessibility barriers by how strongly they affect a user's ability to complete a task. Maintainers assign or confirm severity during triage; reporters do not need to.

| Level | Definition | Example |
|---|---|---|
| Critical | Content or function is completely inaccessible; no workaround exists. | A form submit button has no label and cannot be activated by keyboard. |
| High | Content or function is difficult to access; a cumbersome workaround may exist. | Focus order jumps illogically through a form, making tab navigation confusing. |
| Medium | Accessibility feature is missing or incomplete; task is still completable. | An icon-only button lacks an accessible name for screen readers. |
| Low | Minor improvement opportunity; no significant barrier. | A decorative image has an empty alt attribute rather than alt="". |

### How we respond

After submitting an accessibility issue, you can expect:

• Acknowledgment — a maintainer acknowledges the report within a reasonable timeframe.
• Triage — the issue is labeled, severity is assigned, and it is added to the backlog.
• Status updates — you will be notified when work begins and when a fix is merged.
• Workarounds — if a fix requires time, we may suggest a workaround when one exists.
• Verification — when a fix is ready, we may ask the reporter to verify it resolves the barrier.

We cannot always guarantee specific timelines, but accessibility issues are treated with the same priority as functional bugs of equivalent severity.

## Ownership and maintenance

Accessibility is a shared responsibility across the project. The maintainer team owns the accessibility statement, triages reported issues, and ensures accessibility is considered in architectural decisions.

This document is reviewed periodically and updated as the project evolves. When responsibilities change, ownership transfers with clear documentation.

## Supported environments

We test and support the following environments:

| Category | Supported |
|---|---|
| Browsers | Chrome, Firefox, Safari, Edge — current and previous versions |
| Operating systems | macOS, Windows, Linux, iOS, Android |
| Input methods | Keyboard, mouse, touch, stylus |
| Assistive tech | VoiceOver (macOS/iOS), NVDA (Windows), TalkBack (Android), screen magnifiers |
| Devices | Desktop, laptop, tablet, mobile |

The Rook dashboard is a web application and is the primary focus of accessibility testing. CLI tools (Rook CLI, Huginn) are tested for keyboard operability and terminal compatibility. Combinations not listed here have not been
formally evaluated.

## Known limitations

We are aware of the following accessibility gaps:

• The Rook dashboard has not been audited with a comprehensive screen reader walkthrough. Some components, particularly charts and dynamic content, may not announce state changes clearly to screen reader users.
• Custom TUI components in Huginn may not be fully compatible with all terminal screen readers.
• Error messages and validation feedback in some forms may not be announced to assistive technology in real time.

We are tracking these limitations and addressing them as capacity allows. If you encounter a barrier not listed here, please report it.

## Feedback and improvements

If you have suggestions for improving this statement or our accessibility practices, please open a discussion or issue. Active accessibility barriers should be reported through the process above.

We welcome feedback from users with diverse abilities and experiences. Your perspective helps us build better products.
