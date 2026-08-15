# Source summarization and conversational analysis for URLs

- Status: feature request for owner and team review
- Date: 2026-08-15

## 1. Purpose

This note requests a product capability: Omega should accept a website link or
a YouTube link, ingest the source, summarize it, extract key points or
structured takeaways, let the user ask follow-up questions grounded in that
source, and keep the source and its outputs inside Omega as reusable internal
knowledge objects. The request is deliberately implementation-agnostic: it
states the user-facing capability and a strong first version, and does not
prescribe how transcripts are fetched, pages are parsed, content is stored, or
a retrieval index is built. Decisions rest with the owners; every
consequential claim below carries an [E], [D], or [P] tag.

## 2. Claim categories

- `[E]` Existing system behavior
- `[D]` Existing documented direction
- `[P]` Proposed future direction

[Taxonomy rule 4](taxonomy.md), "Say what is true now, not what is planned,"
is why every consequential claim is tagged. This note is a proposal, so
proposed direction is always labeled as such and verified absences are marked.

## 3. The gap

Omega is a conversation surface with a fetch primitive and thread
summarization, but it has no source-ingestion loop. A `fetch` tool already
retrieves a URL and returns its content as Markdown, gated by the same
per-host network grants as the terminal ([E]; crates/agent/src/tools/
fetch_tool.rs). Thread summarization prompts exist ([E];
`SUMMARIZE_THREAD_PROMPT` and `SUMMARIZE_THREAD_DETAILED_PROMPT` in
crates/agent_settings). What is missing is the product shape around those
primitives: a repeatable workflow that takes a link, produces a summary and
structured takeaways, holds a grounded conversation about the source, and
persists the source and its outputs for later reuse. There is no YouTube
ingestion or transcript handling anywhere in the tree today ([E] verified
absence).

## 4. Requested capability

[P] Omega should let a user provide a website link or a YouTube link and then:

- ingest the source
- summarize the source
- extract key points or structured takeaways
- ask follow-up questions about the source
- keep the conversation grounded in the source material
- save the source and its outputs inside Omega for later reuse

This is a product capability request, not an implementation plan. The value
is the observable workflow: a link goes in, a useful analysis comes out, and
both the source and the analysis remain available inside Omega.

## 5. Website sources

[P] For a website source, the user should be able to:

- summarize the page
- chat with the content of the page
- save the page analysis into their Omega workflow

The existing `fetch` primitive already converts HTML pages to Markdown under
per-host network grants ([E]); the request is the workflow on top: a summary,
a grounded conversation about the page, and a saved analysis.

## 6. YouTube sources

[P] For a YouTube source, the user should be able to:

- summarize the video
- generate structured takeaways
- ask follow-up questions about the video
- identify key sections, key moments, or chapter-style insights where
  appropriate
- save the result inside Omega for later use

No YouTube or transcript capability exists in the tree today ([E] verified
absence). The request does not prescribe how transcript content is obtained;
it states what the user should be able to do with a video link.

## 7. Follow-up conversation grounded in the source

[P] After a source is ingested, follow-up questions should be answered from
the source material, and the conversation should stay grounded in it rather
than drifting into general knowledge. Where the source does not answer a
question, the response should say so instead of inventing an answer. This is
the difference between "chat about a topic" and "chat with a document."

## 8. Reusable internal knowledge objects

[P] An imported source and its outputs should become reusable internal
knowledge objects inside Omega: saved once, discoverable, and usable again in
later threads, briefs, and analysis. The source URL, the summary, the
structured takeaways, and the conversation record belong together as one
object. This is what lets Omega act as a true source-ingestion and analysis
environment rather than only a chat interface.

## 9. Why this matters

[P] Source summarization and conversational analysis would support the
workflows Omega already serves:

- research workflows
- founder and operator workflows
- content production
- meeting preparation
- onboarding and training
- internal analysis and brief generation

For research, a page or video becomes a summarized, queryable source instead
of a paste job. For content production, a source becomes a brief. For meeting
preparation, a source becomes talking points. For onboarding and training,
sources become reusable reference knowledge. This is a high-value, near-term
addition: it directly extends primitives Omega already has ([E] fetch tool,
[E] thread summarization) into a coherent product workflow.

## 10. What a strong first version should enable

[P] A strong first version should let a user, from one thread:

- paste a website link and get a summary of the page
- paste a YouTube link and get a summary of the video with structured
  takeaways
- ask follow-up questions about either source and get answers grounded in
  that source
- identify key sections or moments for video sources where the source
  supports it
- save the source and its analysis, and return to it later
- reuse a saved source in later work without re-pasting the link

## 11. Non-goals / out of scope

This request does not propose:

- any specific transcript-fetching, page-parsing, or storage mechanism
- any retrieval-index or backend architecture
- any change to network-grant behavior for fetching ([E] per-host grants are
  the existing boundary)
- any change to identity, authentication, or the conversation contract

It is a capability request, not a build plan.

## 12. Privacy and source-handling notes

[P] Product-level considerations for the team to weigh, not decisions made
here: fetched content is retrieved under the user's existing network grants
([E]), so source ingestion should behave like any other user-authorized fetch;
saved sources should follow Omega's local data posture and not leak to other
audiences by default; and any transcript-derived content should be handled
with the same privacy expectations as any other imported material.

## 13. Open design questions

1. Should a saved source be scoped to the thread, the project, or the
   account? [P]
2. Where does the saved-source library surface (a panel, a command palette
   action, a thread artifact)? [P]
3. What does "grounded" mean for the first version — strictly quote the
   source, or allow synthesis with clear attribution? [P]
4. For video, is a section or chapter outline part of the first version or a
   follow-up? [P]
5. Should ingestion be synchronous (summary appears in-thread) or async
   (background job with a result artifact)? [P]

## 14. Related documents and evidence

- [taxonomy.md](taxonomy.md): glossary and naming rules; rule 4 is the claim
  discipline this note follows
- [../../crates/agent/src/tools/fetch_tool.rs](../../crates/agent/src/tools/fetch_tool.rs):
  the existing fetch primitive and per-host network grants [E]
- [../../crates/agent_settings/src/prompts/summarize_thread_detailed_prompt.txt](../../crates/agent_settings/src/prompts/summarize_thread_detailed_prompt.txt):
  the existing thread-summarization prompt [E]
- [PRODUCT.md](../../PRODUCT.md): Omega's product shape, users, and design
  principles
