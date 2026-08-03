# Omega Work Domains

Work in Omega is domain-neutral. Software product development is one profile
over Work, not the boundary of the application. A **Work Domain profile** is the
declarative statement of what a domain can contain.

## A domain is data, not a branch {#a-domain-is-data-not-a-branch}

`omega_work_index::WorkDomainProfile` declares, for one canonical `WorkDomain`:

- the Work classes its source can emit;
- the Work states its source can emit;
- which optional fields it admits — assignee, agent delegate, declared
  priority, portfolio, change review, service health;
- whether urgency is **declared** by a person or **observed** from state;
- the vocabulary a surface uses to name a canonical state inside that domain.

`state_label` is vocabulary only. It never changes which canonical state a row
is in, so cross-domain filtering, counting, sorting, and identity stay on the
canonical value.

Every native adapter builds its summary through one constructor, and that
constructor runs `profile.admit`. An adapter cannot opt out of its own domain,
and a domain cannot be widened past the generated All Work contract — a profile
only narrows.

A domain the product has not specified yet is marked `specified: false`. It
admits the full canonical vocabulary, so an unfilled table row fails open
against the table and closed against the contract. Use `admits_field` to refuse
and `declares_field` to decide what a surface renders: only the second is exact,
so an unspecified domain can never silently claim a capability.

## The specified domains {#the-specified-domains}

| Domain | Classes | Urgency | Admitted fields |
| --- | --- | --- | --- |
| General | Task | declared | assignee, agent delegate, declared priority |
| Development | Task, Change, Review, Run, Outcome | declared | assignee, agent delegate, declared priority, portfolio, change review |
| Security | Case, Investigation, Run, Review | declared | declared priority |
| Operations | Job | observed | service health |

Every other canonical domain — CI, deployment, incident, research, design
review, service delivery, and data — is unspecified.

## Operations: the non-software profile {#operations-the-non-software-profile}

The Operations domain projects the runtime services this Omega process operates.
It is deliberately unlike software product development:

- Its Work is a **service**, not a task. Nothing in it is proposed, estimated,
  assigned, reviewed, or merged.
- It admits **no declared priority at all**. Urgency is derived only from the
  last observation, so an unavailable service outranks every declared priority
  in the shared ordering without declaring one.
- Its identity is the service and the scope it serves. A restart, a new
  operating-system process, and a version upgrade all keep one Work identity;
  the same service run for two working folders is two operated services.
- It admits no assignee, no agent delegate, and no portfolio.

The first Operations source is the language services of the current window,
observed in this process. `omega-effectd` is not involved, so this lane
populates whether or not a packaged component can serve All Work.

### Exact observation {#exact-observation}

Absent is not healthy. The projection distinguishes:

- **provisioning** — checking, downloading, or starting, or no process observed;
- **serving** — running with nothing reporting degradation;
- **working** — running with named work in flight;
- **degraded** — still serving while reporting a warning;
- **unavailable** — a reported error or a failed binary, and recoverable;
- **stopped** — deliberately not running;
- **not observed** — no live observation from this window, rendered as a
  missing input rather than as health.

Severity wins over activity: a service that reported an error and is also
indexing is unavailable, not busy. A binary failure that carries no message is
refused rather than turned into an unexplained Inbox row.

## Two things the abstraction could not express {#two-things-the-abstraction-could-not-express}

These are recorded rather than hidden, because they are properties of the
canonical contract and not of one adapter.

1. **The canonical `WorkState` vocabulary is task-shaped and has no
   "degraded".** A service that still serves while reporting a warning has no
   exact canonical state. It maps to the closest admitted value, `Waiting`, and
   the exact observation stays in the row description and in the domain's
   Blocks. The profile's vocabulary table is what keeps that honest at the
   surface; it does not add a state.
2. **An empty source reference still produces a syntactically valid canonical
   reference.** `work:omega:runtime-service:` passes the contract and would
   collapse every unnamed service onto one Work identity, so the adapter refuses
   it. A name that cannot be a canonical reference is likewise refused, never
   rewritten into a different identity than the caller published: an
   unrepresentable service name becomes an explicit opaque digest that is stable
   across restarts, distinct per name, and does not pretend to be the name.

## Blocks come from the profile {#blocks-come-from-the-profile}

`omega_work_detail::default_blocks` composes two things:

- what the **source entity** can show — a Thread has a conversation, a Security
  case has a review;
- what the **Work Domain** declares — a domain that declares service health
  always has a metric and a log to show, whatever entity carries it.

The Operations domain adds no source-entity arm content. Its Blocks come
entirely from its profile, which is the test that the composition works: a
future domain of the same shape needs no new branch.

## Boundary {#boundary}

The Operations lane is a read-only projection. It admits no command, holds no
mutation capability, and is not persisted — a snapshot of which services were
running in a previous session is not an observation of anything, so the lane is
replaced at startup by what this window can actually see.
