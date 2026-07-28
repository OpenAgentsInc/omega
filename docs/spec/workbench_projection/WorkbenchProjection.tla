------------------------ MODULE WorkbenchProjection ------------------------
(***************************************************************************)
(* Bounded model of Omega's logical desktop workbench projection.           *)
(*                                                                         *)
(* The model deliberately excludes layout, pixels, GPUI entities, message  *)
(* text, paths, and tool output. It models only the identities and revisions *)
(* required to decide which surface may render and receive actions.         *)
(***************************************************************************)
EXTENDS Naturals, FiniteSets

CONSTANTS
    Threads,
    Repositories,
    Worktrees,
    PrimaryThread,
    SecondaryThread,
    PrimaryRepository,
    SecondaryRepository,
    PrimaryWorktree,
    SecondaryWorktree,
    MaxGeneration,
    MaxRevision,
    MaxSteps,
    Scenario,
    MutAcceptStaleCompletion,
    MutRestorePreviousThread,
    MutHiddenOwner,
    MutApplyOlderRevision,
    MutKeepMissingWorktree,
    MutNondeterministicFallback,
    DisableStaleCompletionAction,
    DisableSettleAction

ASSUME
    /\ Threads = {PrimaryThread, SecondaryThread}
    /\ PrimaryThread # SecondaryThread
    /\ PrimaryRepository \in Repositories
    /\ SecondaryRepository \in Repositories
    /\ PrimaryRepository # SecondaryRepository
    /\ PrimaryWorktree \in Worktrees
    /\ SecondaryWorktree \in Worktrees
    /\ PrimaryWorktree # SecondaryWorktree
    /\ MaxGeneration \in Nat \ {0}
    /\ MaxRevision \in Nat \ {0}
    /\ MaxSteps \in Nat \ {0}
    /\ Scenario \in {
        "Full",
        "ColdRestore",
        "Reconnect",
        "InvalidFallback",
        "StaleCompletion",
        "HiddenCompletion",
        "HiddenOwner",
        "Persistence",
        "Restore",
        "Fallback"}

NoThread == "NoThread"
NoRepository == "NoRepository"
NoWorktree == "NoWorktree"
NoSurface == "NoSurface"

Surfaces == {"Files", "Git", "Terminal", "Plan"}
WorktreeSurfaces == {"Git", "Terminal"}
BindingSurfaces == {"Files", "Git", "Terminal"}
GenerationValues == 0..MaxGeneration
RevisionValues == 0..MaxRevision
ThreadValues == Threads \union {NoThread}
RepositoryValues == Repositories \union {NoRepository}
WorktreeValues == Worktrees \union {NoWorktree}
SurfaceValues == Surfaces \union {NoSurface}

VARIABLES
    knownThreads,
    activeThread,
    repositoryOf,
    worktreeOf,
    capabilities,
    requestedSurface,
    requestedDock,
    effectiveSurface,
    dockVisible,
    focusOwners,
    renderedBinding,
    actionBinding,
    selectionOwner,
    artifactOwner,
    eventOwner,
    generation,
    pendingLoads,
    acceptedStaleCompletions,
    artifactRevision,
    eventRevision,
    commandViolation,
    persistedSurface,
    persistedDock,
    persistedRepository,
    persistedWorktree,
    persistedGeneration,
    persistedRevision,
    appliedRevision,
    maxSeenRevision,
    connectionPhase,
    restorePendingThread,
    projectionPhase,
    coldRestoreSeen,
    reconnectSeen,
    invalidFallbackSeen,
    staleCompletionSeen,
    hiddenCurrentCompletionSeen,
    restoreViolation,
    quiescing,
    step

vars ==
    <<knownThreads, activeThread, repositoryOf, worktreeOf, capabilities,
      requestedSurface, requestedDock, effectiveSurface, dockVisible,
      focusOwners, renderedBinding, actionBinding, selectionOwner,
      artifactOwner, eventOwner, generation, pendingLoads,
      acceptedStaleCompletions, artifactRevision, eventRevision,
      commandViolation, persistedSurface, persistedDock,
      persistedRepository, persistedWorktree, persistedGeneration,
      persistedRevision, appliedRevision, maxSeenRevision, connectionPhase,
      restorePendingThread, projectionPhase, coldRestoreSeen, reconnectSeen,
      invalidFallbackSeen, staleCompletionSeen, hiddenCurrentCompletionSeen,
      restoreViolation, quiescing, step>>

threadVars ==
    <<knownThreads, repositoryOf, worktreeOf, capabilities, generation>>

selectionVars == <<requestedSurface, requestedDock>>

projectionIdentityVars ==
    <<activeThread, effectiveSurface, dockVisible, focusOwners,
      renderedBinding, actionBinding, selectionOwner, artifactOwner,
      eventOwner>>

asyncVars ==
    <<pendingLoads, acceptedStaleCompletions, artifactRevision, eventRevision>>

persistenceVars ==
    <<persistedSurface, persistedDock, persistedRepository,
      persistedWorktree, persistedGeneration, persistedRevision,
      appliedRevision, maxSeenRevision>>

lifecycleVars == <<connectionPhase, restorePendingThread>>

witnessVars ==
    <<coldRestoreSeen, reconnectSeen, invalidFallbackSeen,
      staleCompletionSeen, hiddenCurrentCompletionSeen, restoreViolation>>

BindingRecords ==
    [thread: ThreadValues,
     repository: RepositoryValues,
     worktree: WorktreeValues,
     surface: SurfaceValues,
     generation: GenerationValues]

LoadRecords ==
    [thread: Threads,
     repository: RepositoryValues,
     worktree: WorktreeValues,
     surface: Surfaces,
     generation: GenerationValues]

EmptyBinding ==
    [thread |-> NoThread,
     repository |-> NoRepository,
     worktree |-> NoWorktree,
     surface |-> NoSurface,
     generation |-> 0]

Rank(surface) ==
    CASE surface = "Files" -> 1
      [] surface = "Git" -> 2
      [] surface = "Terminal" -> 3
      [] surface = "Plan" -> 4
      [] OTHER -> 5

AvailableForState(thread, known, repositories, worktrees, capabilityState) ==
    IF thread \notin known
    THEN {}
    ELSE capabilityState[thread]
        \intersect
        ({"Plan"}
          \union (IF repositories[thread] # NoRepository
                      /\ worktrees[thread] # NoWorktree
                  THEN BindingSurfaces
                  ELSE {}))

FallbackForState(thread, known, repositories, worktrees, capabilityState) ==
    LET available ==
            AvailableForState(
                thread,
                known,
                repositories,
                worktrees,
                capabilityState)
    IN
        IF available = {}
        THEN NoSurface
        ELSE CHOOSE surface \in available :
            \A other \in available : Rank(surface) <= Rank(other)

BadFallbackForState(thread, known, repositories, worktrees, capabilityState) ==
    LET available ==
            AvailableForState(
                thread,
                known,
                repositories,
                worktrees,
                capabilityState)
    IN
        IF available = {}
        THEN NoSurface
        ELSE CHOOSE surface \in available :
            \A other \in available : Rank(surface) >= Rank(other)

SelectedForState(
    thread,
    known,
    repositories,
    worktrees,
    capabilityState,
    requestState) ==
    LET requested == requestState[thread]
        available ==
            AvailableForState(
                thread,
                known,
                repositories,
                worktrees,
                capabilityState)
    IN
        IF requested = NoSurface
        THEN NoSurface
        ELSE IF requested \in available
             THEN requested
             ELSE IF MutNondeterministicFallback
                  THEN BadFallbackForState(
                      thread,
                      known,
                      repositories,
                      worktrees,
                      capabilityState)
                  ELSE FallbackForState(
                      thread,
                      known,
                      repositories,
                      worktrees,
                      capabilityState)

CorrectSelectedForState(
    thread,
    known,
    repositories,
    worktrees,
    capabilityState,
    requestState) ==
    LET requested == requestState[thread]
        available ==
            AvailableForState(
                thread,
                known,
                repositories,
                worktrees,
                capabilityState)
    IN
        IF requested = NoSurface
        THEN NoSurface
        ELSE IF requested \in available
             THEN requested
             ELSE FallbackForState(
                thread,
                known,
                repositories,
                worktrees,
                capabilityState)

BindingForState(thread, surface, repositories, worktrees, generationState) ==
    IF thread = NoThread \/ surface = NoSurface
    THEN EmptyBinding
    ELSE
        [thread |-> thread,
         repository |-> repositories[thread],
         worktree |-> worktrees[thread],
         surface |-> surface,
         generation |-> generationState[thread]]

OtherThread(thread) ==
    IF thread = PrimaryThread THEN SecondaryThread ELSE PrimaryThread

FirstKnownThread(known) ==
    IF PrimaryThread \in known
    THEN PrimaryThread
    ELSE IF SecondaryThread \in known THEN SecondaryThread ELSE NoThread

ProjectWith(
    thread,
    known,
    repositories,
    worktrees,
    capabilityState,
    requestState,
    dockState,
    generationState) ==
    LET selected ==
            SelectedForState(
                thread,
                known,
                repositories,
                worktrees,
                capabilityState,
                requestState)
        showDock == dockState[thread] /\ selected # NoSurface
        binding ==
            BindingForState(
                thread,
                selected,
                repositories,
                worktrees,
                generationState)
    IN
        /\ activeThread' = thread
        /\ effectiveSurface' = selected
        /\ dockVisible' = showDock
        /\ focusOwners' = IF showDock THEN {selected} ELSE {}
        /\ renderedBinding' = binding
        /\ actionBinding' = IF showDock THEN binding ELSE EmptyBinding
        /\ selectionOwner' = thread
        /\ artifactOwner' = thread
        /\ eventOwner' = thread
        /\ projectionPhase' =
            IF selected = NoSurface THEN "Empty" ELSE "Ready"

ProjectExplicit(
    thread,
    surface,
    showDock,
    owner,
    repositories,
    worktrees,
    generationState) ==
    LET binding ==
            BindingForState(
                thread,
                surface,
                repositories,
                worktrees,
                generationState)
    IN
        /\ activeThread' = thread
        /\ effectiveSurface' = surface
        /\ dockVisible' = showDock /\ surface # NoSurface
        /\ focusOwners' =
            IF showDock /\ surface # NoSurface THEN {surface} ELSE {}
        /\ renderedBinding' = binding
        /\ actionBinding' =
            IF showDock /\ surface # NoSurface THEN binding ELSE EmptyBinding
        /\ selectionOwner' = owner
        /\ artifactOwner' = thread
        /\ eventOwner' = thread
        /\ projectionPhase' =
            IF surface = NoSurface THEN "Empty" ELSE "Ready"

ClearProjection ==
    /\ activeThread' = NoThread
    /\ effectiveSurface' = NoSurface
    /\ dockVisible' = FALSE
    /\ focusOwners' = {}
    /\ renderedBinding' = EmptyBinding
    /\ actionBinding' = EmptyBinding
    /\ selectionOwner' = NoThread
    /\ artifactOwner' = NoThread
    /\ eventOwner' = NoThread
    /\ projectionPhase' = "Empty"

InitialRepository(thread) ==
    IF thread = PrimaryThread
    THEN PrimaryRepository
    ELSE SecondaryRepository

InitialWorktree(thread) ==
    IF thread = PrimaryThread
    THEN PrimaryWorktree
    ELSE SecondaryWorktree

InitialRequest(thread) ==
    IF thread = PrimaryThread THEN "Git" ELSE "Terminal"

Init ==
    /\ knownThreads = Threads
    /\ activeThread = PrimaryThread
    /\ repositoryOf = [thread \in Threads |-> InitialRepository(thread)]
    /\ worktreeOf = [thread \in Threads |-> InitialWorktree(thread)]
    /\ capabilities = [thread \in Threads |-> Surfaces]
    /\ requestedSurface = [thread \in Threads |-> InitialRequest(thread)]
    /\ requestedDock = [thread \in Threads |-> TRUE]
    /\ effectiveSurface = InitialRequest(PrimaryThread)
    /\ dockVisible = TRUE
    /\ focusOwners = {InitialRequest(PrimaryThread)}
    /\ renderedBinding =
        BindingForState(
            PrimaryThread,
            InitialRequest(PrimaryThread),
            repositoryOf,
            worktreeOf,
            [thread \in Threads |-> 0])
    /\ actionBinding = renderedBinding
    /\ selectionOwner = PrimaryThread
    /\ artifactOwner = PrimaryThread
    /\ eventOwner = PrimaryThread
    /\ generation = [thread \in Threads |-> 0]
    /\ pendingLoads = {}
    /\ acceptedStaleCompletions = {}
    /\ artifactRevision = [thread \in Threads |-> 0]
    /\ eventRevision = [thread \in Threads |-> 0]
    /\ commandViolation = FALSE
    /\ persistedSurface = requestedSurface
    /\ persistedDock = requestedDock
    /\ persistedRepository = repositoryOf
    /\ persistedWorktree = worktreeOf
    /\ persistedGeneration = generation
    /\ persistedRevision = [thread \in Threads |-> 0]
    /\ appliedRevision = [thread \in Threads |-> 0]
    /\ maxSeenRevision = [thread \in Threads |-> 0]
    /\ connectionPhase = "Online"
    /\ restorePendingThread = NoThread
    /\ projectionPhase = "Ready"
    /\ coldRestoreSeen = FALSE
    /\ reconnectSeen = FALSE
    /\ invalidFallbackSeen = FALSE
    /\ staleCompletionSeen = FALSE
    /\ hiddenCurrentCompletionSeen = FALSE
    /\ restoreViolation = FALSE
    /\ quiescing = FALSE
    /\ step = 0

SwitchThread(thread) ==
    /\ thread \in knownThreads
    /\ thread # activeThread
    /\ ProjectWith(
        thread,
        knownThreads,
        repositoryOf,
        worktreeOf,
        capabilities,
        requestedSurface,
        requestedDock,
        generation)
    /\ UNCHANGED
        <<threadVars, selectionVars, asyncVars, persistenceVars,
          lifecycleVars, witnessVars, commandViolation, quiescing>>

OpenThread(thread) ==
    /\ thread \in Threads \ knownThreads
    /\ LET knownState == knownThreads \union {thread}
       IN
        /\ knownThreads' = knownState
        /\ IF activeThread = NoThread
           THEN ProjectWith(
                thread,
                knownState,
                repositoryOf,
                worktreeOf,
                capabilities,
                requestedSurface,
                requestedDock,
                generation)
           ELSE UNCHANGED <<projectionIdentityVars, projectionPhase>>
    /\ UNCHANGED
        <<repositoryOf, worktreeOf, capabilities, selectionVars,
          generation, asyncVars, persistenceVars, lifecycleVars,
          witnessVars, commandViolation, quiescing>>

CloseThread(thread) ==
    /\ thread \in knownThreads
    /\ LET knownState == knownThreads \ {thread}
           nextThread == FirstKnownThread(knownState)
       IN
        /\ knownThreads' = knownState
        /\ pendingLoads' =
            {load \in pendingLoads : load.thread # thread}
        /\ IF activeThread = thread
           THEN IF nextThread = NoThread
                THEN ClearProjection
                ELSE ProjectWith(
                    nextThread,
                    knownState,
                    repositoryOf,
                    worktreeOf,
                    capabilities,
                    requestedSurface,
                    requestedDock,
                    generation)
           ELSE UNCHANGED <<projectionIdentityVars, projectionPhase>>
    /\ restorePendingThread' =
        IF restorePendingThread = thread THEN NoThread ELSE restorePendingThread
    /\ UNCHANGED
        <<repositoryOf, worktreeOf, capabilities, selectionVars,
          generation, acceptedStaleCompletions, artifactRevision,
          eventRevision, persistenceVars, connectionPhase,
          witnessVars, commandViolation, quiescing>>

RequestSurface(surface) ==
    /\ activeThread \in knownThreads
    /\ surface \in SurfaceValues
    /\ surface \in capabilities[activeThread]
    /\ connectionPhase = "Online" \/ surface = "Plan"
    /\ LET requestState ==
            [requestedSurface EXCEPT ![activeThread] = surface]
           dockState ==
            [requestedDock EXCEPT ![activeThread] = TRUE]
       IN
        /\ requestedSurface' = requestState
        /\ requestedDock' = dockState
        /\ ProjectWith(
            activeThread,
            knownThreads,
            repositoryOf,
            worktreeOf,
            capabilities,
            requestState,
            dockState,
            generation)
    /\ UNCHANGED
        <<threadVars, asyncVars, persistenceVars, lifecycleVars,
          witnessVars, commandViolation, quiescing>>

CollapseDock ==
    /\ activeThread \in knownThreads
    /\ dockVisible
    /\ requestedDock' = [requestedDock EXCEPT ![activeThread] = FALSE]
    /\ requestedSurface' = requestedSurface
    /\ dockVisible' = FALSE
    /\ IF MutHiddenOwner
       THEN
        /\ UNCHANGED <<focusOwners, actionBinding>>
       ELSE
        /\ focusOwners' = {}
        /\ actionBinding' = EmptyBinding
    /\ UNCHANGED
        <<threadVars, activeThread, effectiveSurface, renderedBinding,
          selectionOwner, artifactOwner, eventOwner, projectionPhase, asyncVars,
          persistenceVars, lifecycleVars, witnessVars, commandViolation,
          quiescing>>

ExpandDock ==
    /\ activeThread \in knownThreads
    /\ ~dockVisible
    /\ connectionPhase = "Online"
       \/ requestedSurface[activeThread] = "Plan"
    /\ requestedDock' = [requestedDock EXCEPT ![activeThread] = TRUE]
    /\ requestedSurface' = requestedSurface
    /\ ProjectWith(
        activeThread,
        knownThreads,
        repositoryOf,
        worktreeOf,
        capabilities,
        requestedSurface,
        requestedDock',
        generation)
    /\ UNCHANGED
        <<threadVars, asyncVars, persistenceVars, lifecycleVars,
          witnessVars, commandViolation, quiescing>>

BindRepository(thread, repository) ==
    /\ thread \in knownThreads
    /\ repository \in Repositories
    /\ repositoryOf[thread] # repository
    /\ generation[thread] < MaxGeneration
    /\ LET repositoryState ==
            [repositoryOf EXCEPT ![thread] = repository]
           generationState ==
            [generation EXCEPT ![thread] = @ + 1]
       IN
        /\ repositoryOf' = repositoryState
        /\ generation' = generationState
        /\ IF activeThread = thread
           THEN ProjectWith(
                thread,
                knownThreads,
                repositoryState,
                worktreeOf,
                capabilities,
                requestedSurface,
                requestedDock,
                generationState)
           ELSE UNCHANGED <<projectionIdentityVars, projectionPhase>>
    /\ UNCHANGED
        <<knownThreads, worktreeOf, capabilities, selectionVars,
          asyncVars, persistenceVars, lifecycleVars, witnessVars,
          commandViolation, quiescing>>

ChangeWorktree(thread, worktree) ==
    /\ thread \in knownThreads
    /\ worktree \in Worktrees
    /\ worktreeOf[thread] # worktree
    /\ generation[thread] < MaxGeneration
    /\ LET worktreeState ==
            [worktreeOf EXCEPT ![thread] = worktree]
           generationState ==
            [generation EXCEPT ![thread] = @ + 1]
       IN
        /\ worktreeOf' = worktreeState
        /\ generation' = generationState
        /\ IF activeThread = thread
           THEN ProjectWith(
                thread,
                knownThreads,
                repositoryOf,
                worktreeState,
                capabilities,
                requestedSurface,
                requestedDock,
                generationState)
           ELSE UNCHANGED <<projectionIdentityVars, projectionPhase>>
    /\ UNCHANGED
        <<knownThreads, repositoryOf, capabilities, selectionVars,
          asyncVars, persistenceVars, lifecycleVars, witnessVars,
          commandViolation, quiescing>>

ChangeBinding(thread, repository, worktree) ==
    /\ thread \in knownThreads
    /\ repository \in Repositories
    /\ worktree \in Worktrees
    /\ generation[thread] < MaxGeneration
    /\ LET repositoryState ==
            [repositoryOf EXCEPT ![thread] = repository]
           worktreeState ==
            [worktreeOf EXCEPT ![thread] = worktree]
           generationState ==
            [generation EXCEPT ![thread] = @ + 1]
       IN
        /\ repositoryOf' = repositoryState
        /\ worktreeOf' = worktreeState
        /\ generation' = generationState
        /\ IF activeThread = thread
           THEN ProjectWith(
                thread,
                knownThreads,
                repositoryState,
                worktreeState,
                capabilities,
                requestedSurface,
                requestedDock,
                generationState)
           ELSE UNCHANGED <<projectionIdentityVars, projectionPhase>>
    /\ UNCHANGED
        <<knownThreads, capabilities, selectionVars, asyncVars,
          persistenceVars, lifecycleVars, witnessVars, commandViolation,
          quiescing>>

RemoveWorktree(thread) ==
    /\ thread \in knownThreads
    /\ worktreeOf[thread] # NoWorktree
    /\ generation[thread] < MaxGeneration
    /\ LET worktreeState ==
            [worktreeOf EXCEPT ![thread] = NoWorktree]
           generationState ==
            [generation EXCEPT ![thread] = @ + 1]
       IN
        /\ worktreeOf' = worktreeState
        /\ generation' = generationState
        /\ IF activeThread = thread
           THEN IF MutKeepMissingWorktree
                THEN UNCHANGED <<projectionIdentityVars, projectionPhase>>
                ELSE ProjectWith(
                    thread,
                    knownThreads,
                    repositoryOf,
                    worktreeState,
                    capabilities,
                    requestedSurface,
                    requestedDock,
                    generationState)
           ELSE UNCHANGED <<projectionIdentityVars, projectionPhase>>
    /\ invalidFallbackSeen' =
        (invalidFallbackSeen
         \/ (activeThread = thread /\ requestedSurface[thread] \in WorktreeSurfaces))
    /\ UNCHANGED
        <<knownThreads, repositoryOf, capabilities, selectionVars,
          asyncVars, persistenceVars, lifecycleVars, coldRestoreSeen,
          reconnectSeen, staleCompletionSeen, hiddenCurrentCompletionSeen,
          restoreViolation,
          commandViolation, quiescing>>

RemoveRepository(thread) ==
    /\ thread \in knownThreads
    /\ repositoryOf[thread] # NoRepository
    /\ generation[thread] < MaxGeneration
    /\ LET repositoryState ==
            [repositoryOf EXCEPT ![thread] = NoRepository]
           worktreeState ==
            [worktreeOf EXCEPT ![thread] = NoWorktree]
           generationState ==
            [generation EXCEPT ![thread] = @ + 1]
       IN
        /\ repositoryOf' = repositoryState
        /\ worktreeOf' = worktreeState
        /\ generation' = generationState
        /\ IF activeThread = thread
           THEN ProjectWith(
                thread,
                knownThreads,
                repositoryState,
                worktreeState,
                capabilities,
                requestedSurface,
                requestedDock,
                generationState)
           ELSE UNCHANGED <<projectionIdentityVars, projectionPhase>>
    /\ UNCHANGED
        <<knownThreads, capabilities, selectionVars, asyncVars,
          persistenceVars, lifecycleVars, witnessVars,
          commandViolation, quiescing>>

InvalidateCapability(thread, surface) ==
    /\ thread \in knownThreads
    /\ surface \in capabilities[thread]
    /\ generation[thread] < MaxGeneration
    /\ LET capabilityState ==
            [capabilities EXCEPT ![thread] = @ \ {surface}]
           generationState ==
            [generation EXCEPT ![thread] = @ + 1]
       IN
        /\ capabilities' = capabilityState
        /\ generation' = generationState
        /\ IF activeThread = thread
           THEN ProjectWith(
                thread,
                knownThreads,
                repositoryOf,
                worktreeOf,
                capabilityState,
                requestedSurface,
                requestedDock,
                generationState)
           ELSE UNCHANGED <<projectionIdentityVars, projectionPhase>>
    /\ invalidFallbackSeen' =
        (invalidFallbackSeen
         \/ (activeThread = thread
             /\ requestedSurface[thread] = surface
             /\ surface # NoSurface))
    /\ UNCHANGED
        <<knownThreads, repositoryOf, worktreeOf, selectionVars,
          asyncVars, persistenceVars, lifecycleVars, coldRestoreSeen,
          reconnectSeen, staleCompletionSeen, hiddenCurrentCompletionSeen,
          restoreViolation,
          commandViolation, quiescing>>

BeginLoad ==
    /\ activeThread \in knownThreads
    /\ effectiveSurface \in Surfaces
    /\ Cardinality(pendingLoads) < 1
    /\ artifactRevision[activeThread] < MaxRevision
    /\ eventRevision[activeThread] < MaxRevision
    /\ LET load ==
            [thread |-> activeThread,
             repository |-> repositoryOf[activeThread],
             worktree |-> worktreeOf[activeThread],
             surface |-> effectiveSurface,
             generation |-> generation[activeThread]]
       IN pendingLoads' = pendingLoads \union {load}
    /\ projectionPhase' = "Loading"
    /\ UNCHANGED
        <<threadVars, selectionVars, projectionIdentityVars,
          acceptedStaleCompletions, artifactRevision, eventRevision,
          persistenceVars, lifecycleVars, witnessVars, commandViolation,
          quiescing>>

IsCurrentLoad(load) ==
    /\ load.thread \in knownThreads
    /\ load.repository = repositoryOf[load.thread]
    /\ load.worktree = worktreeOf[load.thread]
    /\ load.surface =
        CorrectSelectedForState(
            load.thread,
            knownThreads,
            repositoryOf,
            worktreeOf,
            capabilities,
            requestedSurface)
    /\ load.generation = generation[load.thread]

CompleteLoad ==
    \E load \in pendingLoads :
        LET stale == ~IsCurrentLoad(load)
        IN
            /\ ~(DisableStaleCompletionAction /\ stale)
            /\ pendingLoads' = pendingLoads \ {load}
            /\ staleCompletionSeen' = (staleCompletionSeen \/ stale)
            /\ hiddenCurrentCompletionSeen' =
                (hiddenCurrentCompletionSeen
                 \/ (~stale /\ load.thread # activeThread))
            /\ acceptedStaleCompletions' =
                IF stale /\ MutAcceptStaleCompletion
                THEN acceptedStaleCompletions \union {load}
                ELSE acceptedStaleCompletions
            /\ IF stale /\ MutAcceptStaleCompletion
               THEN
                /\ activeThread' = activeThread
                /\ effectiveSurface' = load.surface
                /\ dockVisible' = TRUE
                /\ focusOwners' = {load.surface}
                /\ renderedBinding' = load
                /\ actionBinding' = load
                /\ selectionOwner' = load.thread
                /\ artifactOwner' = load.thread
                /\ eventOwner' = load.thread
                /\ projectionPhase' = "Ready"
               ELSE
                /\ IF load.thread = activeThread
                   THEN projectionPhase' = "Ready"
                   ELSE UNCHANGED projectionPhase
                /\ UNCHANGED projectionIdentityVars
            /\ IF stale
               THEN UNCHANGED <<artifactRevision, eventRevision>>
               ELSE
                /\ artifactRevision' =
                    [artifactRevision EXCEPT ![load.thread] = @ + 1]
                /\ eventRevision' =
                    [eventRevision EXCEPT ![load.thread] = @ + 1]
            /\ UNCHANGED
                <<threadVars, selectionVars, persistenceVars,
                  lifecycleVars, coldRestoreSeen, reconnectSeen,
                  invalidFallbackSeen, restoreViolation,
                  commandViolation, quiescing>>

FailLoad ==
    \E load \in pendingLoads :
        /\ pendingLoads' = pendingLoads \ {load}
        /\ IF load.thread = activeThread
           THEN projectionPhase' = "Ready"
           ELSE UNCHANGED projectionPhase
        /\ UNCHANGED
            <<threadVars, selectionVars, projectionIdentityVars,
              acceptedStaleCompletions, artifactRevision, eventRevision,
              persistenceVars, lifecycleVars, witnessVars, commandViolation,
              quiescing>>

Disconnect ==
    /\ connectionPhase \in {"Online", "Stale"}
    /\ connectionPhase' = "Offline"
    /\ restorePendingThread' = restorePendingThread
    /\ projectionPhase' = "Stale"
    /\ UNCHANGED
        <<threadVars, selectionVars, projectionIdentityVars, asyncVars,
          persistenceVars, witnessVars, commandViolation, quiescing>>

BeginReconnect ==
    /\ connectionPhase = "Offline"
    /\ connectionPhase' = "Reconnecting"
    /\ restorePendingThread' = restorePendingThread
    /\ UNCHANGED
        <<threadVars, selectionVars, projectionIdentityVars,
          projectionPhase, asyncVars, persistenceVars, witnessVars,
          commandViolation, quiescing>>

Max2(first, second) == IF first >= second THEN first ELSE second

ReceiveSnapshot(surface, showDock, revision) ==
    /\ connectionPhase \in {"Reconnecting", "Stale"}
    /\ activeThread \in knownThreads
    /\ surface \in SurfaceValues
    /\ showDock \in BOOLEAN
    /\ revision \in RevisionValues
    /\ LET thread == activeThread
           requestState ==
            [requestedSurface EXCEPT ![thread] = surface]
           dockState ==
            [requestedDock EXCEPT ![thread] = showDock]
           shouldApply ==
            (revision > maxSeenRevision[thread]
             \/ MutApplyOlderRevision)
       IN
        /\ maxSeenRevision' =
            [maxSeenRevision EXCEPT
                ![thread] = Max2(@, revision)]
        /\ IF shouldApply
           THEN
            /\ requestedSurface' = requestState
            /\ requestedDock' = dockState
            /\ persistedSurface' =
                [persistedSurface EXCEPT ![thread] = surface]
            /\ persistedDock' =
                [persistedDock EXCEPT ![thread] = showDock]
            /\ persistedRepository' =
                [persistedRepository EXCEPT ![thread] = repositoryOf[thread]]
            /\ persistedWorktree' =
                [persistedWorktree EXCEPT ![thread] = worktreeOf[thread]]
            /\ persistedGeneration' =
                [persistedGeneration EXCEPT ![thread] = generation[thread]]
            /\ persistedRevision' =
                [persistedRevision EXCEPT ![thread] = revision]
            /\ appliedRevision' =
                [appliedRevision EXCEPT ![thread] = revision]
            /\ ProjectWith(
                thread,
                knownThreads,
                repositoryOf,
                worktreeOf,
                capabilities,
                requestState,
                dockState,
                generation)
           ELSE
            /\ UNCHANGED
                <<selectionVars, projectionIdentityVars, projectionPhase,
                  persistedSurface, persistedDock, persistedRepository,
                  persistedWorktree, persistedGeneration,
                  persistedRevision, appliedRevision>>
        /\ connectionPhase' = IF shouldApply THEN "Online" ELSE "Stale"
        /\ restorePendingThread' = restorePendingThread
        /\ reconnectSeen' = (reconnectSeen \/ shouldApply)
    /\ UNCHANGED
        <<threadVars, asyncVars, coldRestoreSeen, invalidFallbackSeen,
          staleCompletionSeen, hiddenCurrentCompletionSeen,
          restoreViolation, commandViolation,
          quiescing>>

PersistSelection(revision) ==
    /\ activeThread \in knownThreads
    /\ revision \in RevisionValues
    /\ LET thread == activeThread
           shouldApply ==
            (revision > maxSeenRevision[thread]
             \/ MutApplyOlderRevision)
       IN
        /\ maxSeenRevision' =
            [maxSeenRevision EXCEPT ![thread] = Max2(@, revision)]
        /\ IF shouldApply
           THEN
            /\ persistedSurface' =
                [persistedSurface EXCEPT
                    ![thread] = requestedSurface[thread]]
            /\ persistedDock' =
                [persistedDock EXCEPT ![thread] = requestedDock[thread]]
            /\ persistedRepository' =
                [persistedRepository EXCEPT ![thread] = repositoryOf[thread]]
            /\ persistedWorktree' =
                [persistedWorktree EXCEPT ![thread] = worktreeOf[thread]]
            /\ persistedGeneration' =
                [persistedGeneration EXCEPT ![thread] = generation[thread]]
            /\ persistedRevision' =
                [persistedRevision EXCEPT ![thread] = revision]
            /\ appliedRevision' =
                [appliedRevision EXCEPT ![thread] = revision]
           ELSE
            /\ UNCHANGED
                <<persistedSurface, persistedDock, persistedRepository,
                  persistedWorktree, persistedGeneration,
                  persistedRevision, appliedRevision>>
    /\ UNCHANGED
        <<threadVars, selectionVars, projectionIdentityVars,
          projectionPhase, asyncVars, lifecycleVars, witnessVars,
          commandViolation, quiescing>>

ColdStart ==
    /\ connectionPhase # "Cold"
    /\ ClearProjection
    /\ connectionPhase' = "Cold"
    /\ restorePendingThread' = activeThread
    /\ UNCHANGED
        <<threadVars, selectionVars, asyncVars, persistenceVars,
          witnessVars, commandViolation, quiescing>>

RestoreExpected(thread) ==
    IF persistedSurface[thread] = NoSurface
    THEN NoSurface
    ELSE IF persistedRepository[thread] = repositoryOf[thread]
            /\ persistedWorktree[thread] = worktreeOf[thread]
            /\ persistedGeneration[thread] = generation[thread]
            /\ persistedSurface[thread]
                \in AvailableForState(
                    thread,
                    knownThreads,
                    repositoryOf,
                    worktreeOf,
                    capabilities)
         THEN persistedSurface[thread]
         ELSE FallbackForState(
            thread,
            knownThreads,
            repositoryOf,
            worktreeOf,
            capabilities)

Restore(thread) ==
    /\ connectionPhase = "Cold"
    /\ activeThread = NoThread
    /\ thread = restorePendingThread
    /\ thread \in knownThreads
    /\ LET source ==
            IF MutRestorePreviousThread
            THEN OtherThread(thread)
            ELSE thread
           expected == RestoreExpected(thread)
           actual == RestoreExpected(source)
           requestState ==
            [requestedSurface EXCEPT ![thread] = actual]
           dockState ==
            [requestedDock EXCEPT
                ![thread] = persistedDock[source] /\ actual # NoSurface]
       IN
        /\ requestedSurface' = requestState
        /\ requestedDock' = dockState
        /\ ProjectExplicit(
            thread,
            actual,
            dockState[thread],
            source,
            repositoryOf,
            worktreeOf,
            generation)
        /\ restoreViolation' =
            (restoreViolation \/ source # thread \/ actual # expected)
    /\ connectionPhase' = "Online"
    /\ restorePendingThread' = NoThread
    /\ coldRestoreSeen' = TRUE
    /\ UNCHANGED
        <<threadVars, asyncVars, persistenceVars, reconnectSeen,
          invalidFallbackSeen, staleCompletionSeen,
          hiddenCurrentCompletionSeen, commandViolation,
          quiescing>>

RouteCommand ==
    /\ focusOwners # {}
    /\ connectionPhase = "Online"
    /\ commandViolation' =
        (commandViolation
         \/ ~dockVisible
         \/ actionBinding # renderedBinding
         \/ actionBinding.thread # activeThread
         \/ actionBinding.repository # repositoryOf[activeThread]
         \/ actionBinding.worktree # worktreeOf[activeThread]
         \/ actionBinding.surface # effectiveSurface
         \/ actionBinding.generation # generation[activeThread])
    /\ UNCHANGED
        <<threadVars, selectionVars, projectionIdentityVars,
          projectionPhase, asyncVars, persistenceVars, lifecycleVars,
          witnessVars, quiescing>>

Converged ==
    IF activeThread = NoThread
    THEN
        /\ effectiveSurface = NoSurface
        /\ ~dockVisible
        /\ focusOwners = {}
        /\ actionBinding = EmptyBinding
    ELSE
        /\ activeThread \in knownThreads
        /\ requestedSurface[activeThread] = effectiveSurface
        /\ requestedDock[activeThread] = (effectiveSurface # NoSurface)
        /\ dockVisible = (effectiveSurface # NoSurface)
        /\ focusOwners =
            IF effectiveSurface = NoSurface THEN {} ELSE {effectiveSurface}
        /\ selectionOwner = activeThread
        /\ artifactOwner = activeThread
        /\ eventOwner = activeThread
        /\ persistedSurface[activeThread] = effectiveSurface
        /\ persistedDock[activeThread] = dockVisible
        /\ persistedRepository[activeThread] = repositoryOf[activeThread]
        /\ persistedWorktree[activeThread] = worktreeOf[activeThread]
        /\ persistedGeneration[activeThread] = generation[activeThread]
        /\ persistedRevision[activeThread] = MaxRevision
        /\ appliedRevision[activeThread] = MaxRevision
        /\ maxSeenRevision[activeThread] = MaxRevision

Quiesce ==
    /\ ~quiescing
    /\ quiescing' = TRUE
    /\ UNCHANGED
        <<threadVars, selectionVars, projectionIdentityVars,
          projectionPhase, asyncVars, persistenceVars, lifecycleVars,
          witnessVars, commandViolation>>

Settle ==
    /\ quiescing
    /\ ~DisableSettleAction
    /\ ~Converged
    /\ activeThread \in knownThreads
    /\ LET thread == activeThread
           selected ==
            SelectedForState(
                thread,
                knownThreads,
                repositoryOf,
                worktreeOf,
                capabilities,
                requestedSurface)
           requestState ==
            [requestedSurface EXCEPT ![thread] = selected]
           dockState ==
            [requestedDock EXCEPT ![thread] = selected # NoSurface]
       IN
        /\ requestedSurface' = requestState
        /\ requestedDock' = dockState
        /\ ProjectWith(
            thread,
            knownThreads,
            repositoryOf,
            worktreeOf,
            capabilities,
            requestState,
            dockState,
            generation)
        /\ persistedSurface' =
            [persistedSurface EXCEPT ![thread] = selected]
        /\ persistedDock' =
            [persistedDock EXCEPT ![thread] = selected # NoSurface]
        /\ persistedRepository' =
            [persistedRepository EXCEPT ![thread] = repositoryOf[thread]]
        /\ persistedWorktree' =
            [persistedWorktree EXCEPT ![thread] = worktreeOf[thread]]
        /\ persistedGeneration' =
            [persistedGeneration EXCEPT ![thread] = generation[thread]]
        /\ persistedRevision' =
            [persistedRevision EXCEPT ![thread] = MaxRevision]
        /\ appliedRevision' =
            [appliedRevision EXCEPT ![thread] = MaxRevision]
        /\ maxSeenRevision' =
            [maxSeenRevision EXCEPT ![thread] = MaxRevision]
    /\ UNCHANGED
        <<threadVars, asyncVars, lifecycleVars, witnessVars,
          commandViolation, quiescing>>

FullExternalNext ==
    \/ \E thread \in Threads : SwitchThread(thread)
    \/ \E thread \in Threads : OpenThread(thread)
    \/ \E thread \in Threads : CloseThread(thread)
    \/ \E surface \in SurfaceValues : RequestSurface(surface)
    \/ CollapseDock
    \/ ExpandDock
    \/ \E thread \in Threads, repository \in Repositories :
        BindRepository(thread, repository)
    \/ \E thread \in Threads, worktree \in Worktrees :
        ChangeWorktree(thread, worktree)
    \/ \E thread \in Threads, repository \in Repositories,
          worktree \in Worktrees :
        ChangeBinding(thread, repository, worktree)
    \/ \E thread \in Threads : RemoveWorktree(thread)
    \/ \E thread \in Threads : RemoveRepository(thread)
    \/ \E thread \in Threads, surface \in Surfaces :
        InvalidateCapability(thread, surface)
    \/ BeginLoad
    \/ CompleteLoad
    \/ FailLoad
    \/ Disconnect
    \/ BeginReconnect
    \/ \E surface \in SurfaceValues, showDock \in BOOLEAN,
          revision \in RevisionValues :
        ReceiveSnapshot(surface, showDock, revision)
    \/ \E revision \in RevisionValues : PersistSelection(revision)
    \/ ColdStart
    \/ \E thread \in Threads : Restore(thread)
    \/ RouteCommand

ExternalNext ==
    CASE Scenario = "Full" -> FullExternalNext
      [] Scenario = "ColdRestore" ->
            ColdStart \/ \E thread \in Threads : Restore(thread)
      [] Scenario = "Reconnect" ->
            \/ Disconnect
            \/ BeginReconnect
            \/ \E surface \in SurfaceValues, showDock \in BOOLEAN,
                  revision \in RevisionValues :
                ReceiveSnapshot(surface, showDock, revision)
      [] Scenario = "InvalidFallback" ->
            \/ \E thread \in Threads : RemoveWorktree(thread)
            \/ \E thread \in Threads, surface \in Surfaces :
                InvalidateCapability(thread, surface)
      [] Scenario = "StaleCompletion" ->
            \/ BeginLoad
            \/ \E thread \in Threads, repository \in Repositories,
                  worktree \in Worktrees :
                ChangeBinding(thread, repository, worktree)
            \/ CompleteLoad
      [] Scenario = "HiddenCompletion" ->
            \/ BeginLoad
            \/ \E thread \in Threads : SwitchThread(thread)
            \/ CompleteLoad
      [] Scenario = "HiddenOwner" -> CollapseDock
      [] Scenario = "Persistence" ->
            \/ Disconnect
            \/ BeginReconnect
            \/ \E surface \in SurfaceValues, showDock \in BOOLEAN,
                  revision \in RevisionValues :
                ReceiveSnapshot(surface, showDock, revision)
      [] Scenario = "Restore" ->
            ColdStart \/ \E thread \in Threads : Restore(thread)
      [] Scenario = "Fallback" ->
            \/ \E thread \in Threads : RemoveWorktree(thread)
            \/ \E thread \in Threads, surface \in Surfaces :
                InvalidateCapability(thread, surface)

AdvanceExternal ==
    /\ step < MaxSteps
    /\ ~quiescing
    /\ ExternalNext
    /\ step' = step + 1

AdvanceQuiesce ==
    /\ step + 1 < MaxSteps
    /\ Quiesce
    /\ step' = step + 1

AdvanceSettle ==
    /\ step < MaxSteps
    /\ Settle
    /\ step' = step + 1

Next ==
    \/ AdvanceExternal
    \/ AdvanceQuiesce
    \/ AdvanceSettle

Spec == Init /\ [][Next]_vars /\ WF_vars(AdvanceSettle)

TypeOK ==
    /\ knownThreads \subseteq Threads
    /\ activeThread \in ThreadValues
    /\ repositoryOf \in [Threads -> RepositoryValues]
    /\ worktreeOf \in [Threads -> WorktreeValues]
    /\ capabilities \in [Threads -> SUBSET Surfaces]
    /\ requestedSurface \in [Threads -> SurfaceValues]
    /\ requestedDock \in [Threads -> BOOLEAN]
    /\ effectiveSurface \in SurfaceValues
    /\ dockVisible \in BOOLEAN
    /\ focusOwners \subseteq Surfaces
    /\ renderedBinding \in BindingRecords
    /\ actionBinding \in BindingRecords
    /\ selectionOwner \in ThreadValues
    /\ artifactOwner \in ThreadValues
    /\ eventOwner \in ThreadValues
    /\ generation \in [Threads -> GenerationValues]
    /\ pendingLoads \subseteq LoadRecords
    /\ acceptedStaleCompletions \subseteq LoadRecords
    /\ artifactRevision \in [Threads -> RevisionValues]
    /\ eventRevision \in [Threads -> RevisionValues]
    /\ commandViolation \in BOOLEAN
    /\ persistedSurface \in [Threads -> SurfaceValues]
    /\ persistedDock \in [Threads -> BOOLEAN]
    /\ persistedRepository \in [Threads -> RepositoryValues]
    /\ persistedWorktree \in [Threads -> WorktreeValues]
    /\ persistedGeneration \in [Threads -> GenerationValues]
    /\ persistedRevision \in [Threads -> RevisionValues]
    /\ appliedRevision \in [Threads -> RevisionValues]
    /\ maxSeenRevision \in [Threads -> RevisionValues]
    /\ connectionPhase \in {
        "Cold", "Online", "Offline", "Reconnecting", "Stale"}
    /\ restorePendingThread \in ThreadValues
    /\ projectionPhase \in {"Empty", "Loading", "Ready", "Stale"}
    /\ coldRestoreSeen \in BOOLEAN
    /\ reconnectSeen \in BOOLEAN
    /\ invalidFallbackSeen \in BOOLEAN
    /\ staleCompletionSeen \in BOOLEAN
    /\ hiddenCurrentCompletionSeen \in BOOLEAN
    /\ restoreViolation \in BOOLEAN
    /\ quiescing \in BOOLEAN
    /\ step \in 0..MaxSteps

Inv_BindingSafety ==
    /\ ~commandViolation
    /\ IF effectiveSurface = NoSurface
       THEN renderedBinding = EmptyBinding
       ELSE
        /\ activeThread \in knownThreads
        /\ renderedBinding =
            BindingForState(
                activeThread,
                effectiveSurface,
                repositoryOf,
                worktreeOf,
                generation)
    /\ IF focusOwners = {}
       THEN actionBinding = EmptyBinding
       ELSE actionBinding = renderedBinding

Inv_SelectionValidity ==
    IF activeThread = NoThread
    THEN effectiveSurface = NoSurface
    ELSE
        /\ activeThread \in knownThreads
        /\ effectiveSurface =
            CorrectSelectedForState(
                activeThread,
                knownThreads,
                repositoryOf,
                worktreeOf,
                capabilities,
                requestedSurface)
        /\ (effectiveSurface = NoSurface
            \/ effectiveSurface
                \in AvailableForState(
                    activeThread,
                    knownThreads,
                    repositoryOf,
                    worktreeOf,
                    capabilities))

Inv_SingleOwner ==
    /\ Cardinality(focusOwners) <= 1
    /\ IF dockVisible
       THEN
        /\ effectiveSurface # NoSurface
        /\ focusOwners = {effectiveSurface}
        /\ actionBinding = renderedBinding
       ELSE
        /\ focusOwners = {}
        /\ actionBinding = EmptyBinding

Inv_StaleCompletionImmunity ==
    acceptedStaleCompletions = {}

Inv_ThreadIsolation ==
    IF activeThread = NoThread
    THEN
        /\ selectionOwner = NoThread
        /\ artifactOwner = NoThread
        /\ eventOwner = NoThread
    ELSE
        /\ selectionOwner = activeThread
        /\ artifactOwner = activeThread
        /\ eventOwner = activeThread
        /\ renderedBinding.thread =
            IF effectiveSurface = NoSurface THEN NoThread ELSE activeThread
        /\ actionBinding.thread =
            IF focusOwners = {} THEN NoThread ELSE activeThread

Inv_PersistenceMonotonicity ==
    /\ persistedRevision = appliedRevision
    /\ appliedRevision = maxSeenRevision

Inv_RestoreFidelity == ~restoreViolation

Safety ==
    /\ TypeOK
    /\ Inv_BindingSafety
    /\ Inv_SelectionValidity
    /\ Inv_SingleOwner
    /\ Inv_StaleCompletionImmunity
    /\ Inv_ThreadIsolation
    /\ Inv_PersistenceMonotonicity
    /\ Inv_RestoreFidelity

EventualConvergence == quiescing ~> Converged

BoundedPending == Cardinality(pendingLoads) <= 1

(***************************************************************************)
(* Intentionally false invariants. A reachability run succeeds by violating *)
(* the selected probe. If one remains true, the corresponding path is dead. *)
(***************************************************************************)
Probe_ColdRestore_Unreached == ~coldRestoreSeen
Probe_Reconnect_Unreached == ~reconnectSeen
Probe_InvalidFallback_Unreached == ~invalidFallbackSeen
Probe_StaleCompletion_Unreached == ~staleCompletionSeen
Probe_HiddenCurrentCompletion_Unreached == ~hiddenCurrentCompletionSeen

=============================================================================
