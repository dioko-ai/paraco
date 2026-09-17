# Portable Application Runtime and Cloud Orchestration

## Repository Scope

This repository contains Paraco's open-source runtime and application framework.
The initial implementation targets standalone operation on one machine, without
a cloud account or service dependency.

This document describes the broader product vision. Cloud orchestration and
managed hosting are future work, outside this repository's initial implementation.
The public control protocol, connector, and provider interfaces remain intended
open-source components, but are deferred until the standalone foundation works.

## Initial Technical Decisions

- The Paraco runtime and CLI will be implemented in Rust.
- Deno will be the first supported application execution runtime.
- The initial local execution backend will use Rust-supervised Deno subprocesses.
  OCI containers may be supported as an additional backend later.
- Portable applications will expose TypeScript request/event handlers using
  standard web APIs and Paraco capability interfaces. Host adapters own runtime
  entry points such as `Deno.serve` and provider-specific worker handlers.
- Portable application code must not assume a local filesystem, subprocesses,
  or an always-running background loop. Device-specific applications may request
  these capabilities with explicit deployment compatibility constraints.
- AI access will use a shared proxy capability that resolves provider, model,
  and user-authorized credentials without exposing those credentials to apps.
- Initial applications will be HTTP apps exporting `fetch(request, context)`.
  Scheduled jobs and background task entry points are deferred.
- A minimal `paraco.json` manifest will declare app identity, entry point, and
  requested capabilities. Capability requests are distinct from user grants.
- AI access will support both a Paraco SDK and an OpenAI-compatible HTTP endpoint,
  sharing routing and authorization. Initial HTTP compatibility will target
  `/v1/chat/completions`, including streaming, with the supported subset documented.

### First Implementation Milestone

`paraco run ./examples/hello` validates the manifest, launches the application
through a Deno host adapter, serves HTTP on localhost, forwards application logs,
and stops and reaps the subprocess on Ctrl+C. The example returns
`Hello from Paraco` and requires no AI credentials or capabilities.

Verification covers a successful HTTP response, invalid manifests, failed
startup, and shutdown without a leftover application process. The initial
implementation consists of one Rust crate, one Deno adapter, and one example.
The AI proxy is a subsequent milestone, beginning with a fake provider for
routing and authorization verification without paid requests.

## 1. Purpose

Build a portable application runtime that allows users to create, install, run, manage, and optionally host small applications through a consistent experience across local machines, self-hosted environments, and managed cloud infrastructure.

The platform is intended primarily for:

- personal productivity applications
- internal tools
- automations
- dashboards
- scheduled jobs
- AI-generated utilities
- small web applications
- lightweight services
- device-connected tools

The platform consists of two major product layers:

1. **Open-source runtime and application framework**
2. **Proprietary cloud orchestration and managed hosting service**

The core principle is:

> Every runtime is independently useful. Cloud orchestration is optional and additive.

A user should be able to install the open-source runtime on a single computer, run a collection of applications locally, and never create a cloud account.

At the other end of the spectrum, a user or organization should be able to connect multiple devices and hosted environments to the commercial cloud service and manage a mixture of local, self-hosted, and cloud-hosted applications through a common control plane.

The same conceptual application model should span all of these scenarios.

---

# 2. Product Thesis

AI has made creating small custom software dramatically easier, but the operational model around that software has not become proportionally easier.

Even a small utility often needs:

- process management
- startup behavior
- routing
- configuration
- persistence
- scheduling
- secrets
- logging
- health monitoring
- notifications
- updates
- deployment
- access control

The platform exists to make these concerns reusable.

A user should be able to create a useful application, run it permanently on their own computer, and later move or extend it into a shared or hosted environment without rebuilding its operational foundation.

The open-source runtime provides independence.

The cloud control plane provides orchestration.

Managed hosting provides convenience.

The application model ties all three together.

---

# 3. Product Goals

The platform should:

- provide a reusable runtime for small applications
- eliminate duplicated infrastructure across individual tools
- provide a consistent application lifecycle
- make applications easy to install and remove
- provide an excellent local-only experience
- support optional cloud orchestration
- support local, self-hosted, and managed execution
- allow applications to move between compatible environments without unnecessary rewrites
- provide a common management experience across environments
- preserve local autonomy and ownership
- support both individuals and teams
- provide a framework that AI coding agents can easily target
- make production deployment materially simpler than deploying arbitrary application infrastructure
- allow users to adopt the commercial service because it is convenient, not because the runtime is artificially crippled without it

---

# 4. Non-Goals

The platform is not intended to replace:

- Kubernetes
- arbitrary container orchestration
- general-purpose cloud infrastructure management
- virtual machines
- enterprise endpoint management
- unrestricted PaaS platforms
- full desktop application frameworks
- IDEs
- configuration management systems
- general-purpose infrastructure-as-code platforms

Managed hosting should remain optimized for applications conforming to the platform application model.

The platform should avoid evolving into a generic "run anything" hosting service.

---

# 5. Open-Source and Commercial Boundary

## 5.1 Core Principle

The division between the open-source project and proprietary cloud should follow this rule:

> Everything required to create, install, run, and manage applications on an individual runtime is open source. Everything required to coordinate runtimes, users, deployments, and infrastructure as an operated service may be proprietary.

The commercial offering should primarily sell:

- coordination
- collaboration
- centralized management
- hosted infrastructure
- convenience
- organizational controls
- operated services

It should not sell basic usability of the runtime.

---

## 5.2 Open-Source Runtime and Framework

The open-source project should include everything needed for a complete standalone runtime experience.

This includes:

- runtime daemon
- application execution
- application lifecycle management
- application manifest and specification
- SDK
- capability interfaces
- local routing
- local scheduler
- local configuration
- local persistence
- local secrets
- local logging
- local health monitoring
- local notifications
- package installation
- package removal
- local updates
- local permissions
- local control interface
- runtime diagnostics
- cloud connector client
- desired-state reconciliation logic
- deployment provider interfaces
- public control-plane protocol definitions

A user should be able to fork the open-source project, disconnect permanently from the commercial service, and still have an excellent local application platform.

---

## 5.3 Proprietary Cloud

The proprietary service may include:

- workspace management
- organization management
- user and team management
- centralized authentication
- role and access management
- fleet management
- centralized desired state
- deployment orchestration
- managed hosting
- centralized secrets management
- cross-runtime observability
- hosted logs and metrics
- hosted application catalog
- hosted domains
- hosted access control
- backups
- managed recovery
- usage metering
- billing
- organizational policy
- cloud control interface
- commercial infrastructure integrations

The commercial service should coordinate runtimes rather than replacing their core execution functionality.

---

## 5.4 Open Protocols

The protocols required for runtimes to interact with a control plane should be public.

Public contracts should include concepts such as:

- runtime identity
- runtime registration
- runtime capabilities
- observed runtime state
- desired runtime state
- applications
- deployments
- versions
- configuration references
- health
- orchestration messages
- provider interfaces

The official commercial cloud may provide the primary implementation of these protocols, but the runtime should not be technically restricted to communicating only with the official service.

---

## 5.5 Deployment Provider Extensibility

The mechanism used to support deployment targets should be extensible.

The open-source framework should define the interface by which deployment environments participate in the system.

This should allow support for:

- local machines
- Linux servers
- private infrastructure
- third-party cloud providers
- community deployment targets
- official managed hosting

The commercial managed hosting implementation may remain proprietary.

---

# 6. Core Design Principles

## 6.1 Local-first, not cloud-dependent

The local runtime must be fully usable without:

- an account
- an internet connection
- a subscription
- a workspace
- the commercial cloud

Cloud integration should add capabilities rather than unlock basic runtime functionality.

---

## 6.2 The Cloud Enhances the Runtime

Connecting a runtime to the cloud should make it easier to:

- coordinate
- deploy
- share
- observe
- update
- administer
- collaborate

It should not turn the runtime into a thin client.

---

## 6.3 Portable Application Model

Applications should describe what they require in platform-level terms rather than depending directly on a specific deployment technology.

Applications may request capabilities such as:

- web interface
- persistent storage
- relational storage
- object storage
- configuration
- secrets
- scheduler
- background work
- notifications
- logging
- health reporting
- network access

Different environments may satisfy those capabilities differently.

---

## 6.4 Deployment Location Is Independent of Management

Where an application executes and where it is managed are separate concerns.

An application may be:

- local and locally managed
- local and cloud-managed
- self-hosted and locally managed
- self-hosted and cloud-managed
- commercially hosted and cloud-managed

---

## 6.5 Execution Is Independent of Accessibility

The platform must distinguish among:

- where an application executes
- who manages it
- who can access it

An application may be:

- device-only
- private
- network-accessible
- workspace-accessible
- authenticated over the internet
- public

These are independent dimensions.

---

## 6.6 Transparent Ownership

The system should make clear:

- where applications run
- where configuration lives
- where data lives
- where secrets live
- who manages the application
- who can access the application
- whether cloud functionality is required

Cloud integration should not silently transfer ownership of local applications or application data.

---

## 6.7 Graceful Cloud Disconnection

A local application that does not inherently require cloud functionality should continue operating if:

- the device loses internet access
- the cloud service is unavailable
- the runtime is removed from a workspace
- the user stops using the commercial service

Only explicitly cloud-dependent functionality should stop.

---

## 6.8 Open Runtime, Operated Cloud

The project should avoid artificial feature gating inside the runtime.

The primary commercial boundary should be:

> You operate it yourself, or we operate and coordinate it for you.

---

# 7. Conceptual Architecture

The platform consists of five primary layers:

1. Application Model
2. Runtime
3. Control Protocol
4. Workspace and Cloud Control Plane
5. Execution and Hosting Environments

---

# 8. Application Model

The Application Model defines the contract applications follow.

It describes:

- identity
- version
- lifecycle
- interfaces
- requested capabilities
- configuration
- storage requirements
- permissions
- health
- commands
- deployability constraints
- portability requirements

The application contract should remain consistent across environments wherever possible.

---

# 9. Runtime

A runtime is an independently useful execution environment capable of hosting platform applications.

A runtime may exist on:

- a laptop
- a desktop
- a workstation
- a home server
- a private server
- company infrastructure
- managed hosting
- supported third-party infrastructure

The runtime owns actual application execution.

The runtime must be capable of operating independently from the cloud control plane.

---

# 10. Control Protocol

The Control Protocol defines how an optional control plane communicates with runtimes.

It supports concepts such as:

- runtime registration
- capability discovery
- desired state
- observed state
- deployment intent
- health reporting
- version reporting
- configuration coordination
- application inventory

The protocol should be part of the open platform contract.

The official cloud control plane is one implementation of a controller using this protocol.

---

# 11. Cloud Control Plane

The Cloud Control Plane coordinates multiple runtimes, environments, applications, users, and deployments.

It should provide:

- centralized visibility
- centralized desired state
- workspace management
- deployment orchestration
- access control
- organizational policy
- hosted services

The control plane should express intent.

Runtimes remain responsible for execution.

---

# 12. Workspace

A Workspace is the primary cloud-level organizational boundary.

A workspace may contain:

- users
- teams
- devices
- runtimes
- environments
- applications
- deployments
- access policies
- organization settings

A workspace may represent:

- one individual
- a household
- a development team
- a company
- another organization

---

# 13. Major Platform Modules

## 13.1 Runtime Core

Responsible for:

- application execution
- lifecycle management
- capability delivery
- local state
- health
- runtime diagnostics
- local control

This module is open source.

---

## 13.2 Application Manager

Responsible for:

- application discovery
- installation
- removal
- activation
- enablement
- version awareness
- permissions
- compatibility
- metadata
- health

This module is open source.

---

## 13.3 Application Execution Layer

Responsible for:

- application startup
- application shutdown
- isolation
- lifecycle events
- restart behavior
- health monitoring
- background execution

Application failures should not destabilize:

- the runtime
- unrelated applications
- the local control interface

This module is open source.

---

## 13.4 Web Gateway

Provides consistent application web access.

Responsibilities include:

- routing
- stable application addresses
- application isolation
- local access
- environment-aware access

The local implementation is open source.

Hosted routing infrastructure may be commercial.

---

## 13.5 Capability Layer

Provides common services to applications.

Initial capability categories may include:

- configuration
- secrets
- storage
- scheduling
- notifications
- logging
- health
- network access
- application data
- proxied AI access

Device runtimes may additionally expose capabilities such as:

- filesystem interaction
- clipboard
- local process integration
- native notifications
- local application interaction
- device hardware

Capability contracts should be open source.

Specific commercial cloud-backed implementations may be proprietary.

---

### 13.5.1 AI Proxy Capability

Applications must access AI providers through a common Paraco AI capability.
Each application process, container, or worker uses the same logical interface;
the host adapter supplies access to the appropriate proxy implementation.

The proxy is responsible for:

- authenticating the calling application or deployment
- enforcing the user's AI credential grants for that caller
- resolving the provider and model for each request
- selecting an authorized credential and calling the provider
- returning responses through the common capability interface

Users configure provider credentials centrally for the runtime and choose which
credentials each application or deployment may use. Applications receive scoped
access to the proxy, not the underlying provider credentials. Sharing the AI
interface does not grant every application access to every credential.

#### Provider and Model Selection

Callers may omit both provider and model, specify a provider, or explicitly pin
a provider/model pair, including hardcoding that pair in application code.

- When both are omitted, the framework resolves them from user-configured
  deployment/application defaults, followed by runtime defaults, within the
  caller's authorized credential set.
- When only a provider is specified, the framework uses the configured default
  model for that provider and an authorized credential for it.
- When both are specified, the proxy honors that exact pair if permitted.
- A model without a provider must resolve unambiguously through configured
  routing; otherwise the proxy returns a clear error requiring a provider.

Explicit selection never bypasses credential grants or proxy authorization.
If no permitted route or credential is available, the request fails clearly.
The proxy must not silently substitute a different provider or model for an
explicitly pinned selection. Responses should identify the provider and model
actually used, without exposing credentials.

#### Local and Hosted Operation

The standalone AI proxy and provider adapter interfaces are open source and
must not require the commercial Paraco service. Calls to external AI providers
still require connectivity to those providers; local providers may be supported
through the same interface.

Future worker deployments use the same application-facing contract with a
reachable, authenticated proxy. Connecting or deploying an app must not
automatically copy local provider credentials into a hosted environment.

Provider secrets must not appear in application responses or logs. Prompt and
response content must not be logged by default.

The proxy is the required framework path for AI access. Enforcing a prohibition
on direct provider calls additionally requires network restrictions in the
execution backend; a supervised subprocess alone does not establish that boundary.

---

## 13.6 Configuration and Settings

The system should distinguish among:

### Application Configuration

Behavior inherent to the application.

### Deployment Configuration

Behavior specific to one deployed instance.

### Runtime Configuration

Behavior of a specific runtime.

### Workspace Configuration

Organization-wide management and policy.

These concerns should remain distinct.

---

## 13.7 Local Control Interface

Every runtime should include a complete local management interface.

It should support:

- applications
- health
- settings
- schedules
- logs
- permissions
- runtime status
- installation
- updates
- diagnostics

This interface is part of the open-source product.

It should never feel like a crippled version of the cloud service.

---

## 13.8 Cloud Control Interface

The commercial cloud provides a broader management interface for:

- workspaces
- applications
- runtimes
- devices
- environments
- deployments
- users
- access
- policy
- hosted services
- fleet health

The local and cloud interfaces should share terminology and mental models.

The cloud interface itself may remain proprietary.

---

## 13.9 Cloud Connector

An optional runtime component connects a runtime to a control plane.

It should support:

- runtime registration
- secure communication
- state synchronization
- desired-state receipt
- observed-state reporting

The connector and runtime-side reconciliation logic should be open source.

---

## 13.10 Desired-State Reconciler

The runtime should be capable of receiving desired state from a control plane and reconciling its actual state toward that intent.

Examples include:

- application should exist
- application should be enabled
- application should run a particular version
- configuration should match intended state
- application should be removed

The reconciler runs locally and is open source.

---

## 13.11 Workspace Service

The commercial service manages:

- workspace state
- membership
- roles
- connected runtimes
- environments
- application inventory
- organizational structure

This is proprietary cloud functionality.

---

## 13.12 Fleet Management

Provides centralized visibility into multiple runtimes.

It should understand:

- runtime identity
- ownership
- connectivity
- version
- health
- supported capabilities
- installed applications
- deployment status

Fleet management is commercial cloud functionality.

---

## 13.13 Deployment Manager

Coordinates deployments across supported environments.

A deployment represents an application instance running on a specific execution target.

An application may have multiple deployments, such as:

- local development
- staging
- production
- personal laptop
- private server

Central orchestration is commercial functionality.

The runtime-side deployment contracts and provider interfaces should remain open.

---

## 13.14 Deployment Providers

Deployment providers represent environments capable of hosting applications.

Potential categories include:

- local runtime
- self-hosted runtime
- third-party cloud
- managed cloud
- specialized execution environment

The provider interface should be open and extensible.

Official hosted provider implementations may be proprietary.

Community providers should be possible.

---

## 13.15 Managed Hosting

Managed Hosting provides platform-operated execution environments.

It may provide:

- application execution
- persistence
- networking
- access
- runtime capabilities
- availability
- backups
- recovery
- scaling
- observability

Managed hosting is a commercial service.

---

## 13.16 Bring-Your-Own Infrastructure

Users should be able to connect infrastructure they operate themselves.

The platform may coordinate it while preserving customer infrastructure ownership.

This may include:

- private servers
- cloud-hosted machines
- company infrastructure
- supported external platforms

---

## 13.17 Authentication and Access

Authentication should remain separate from application execution.

Applications may be:

- local-only
- private
- workspace-restricted
- externally authenticated
- public

Local applications should not require cloud authentication.

Cloud-managed access control may be part of the commercial service.

---

## 13.18 Secrets Management

Secrets should be scoped appropriately to:

- a local runtime
- a deployment
- an environment
- a workspace

The open-source runtime should support local secrets.

The cloud may provide managed cross-environment secrets as a commercial capability.

---

## 13.19 Observability

Each runtime should provide local observability.

This includes:

- application health
- logs
- activity
- scheduled executions
- failures
- runtime status

The cloud may provide aggregated observability across:

- runtimes
- environments
- deployments
- users
- teams

Local observability is open source.

Cross-runtime aggregation is commercial.

---

## 13.20 Application Catalog

The platform may provide support for discovering installable applications.

The ecosystem may contain:

- first-party apps
- community apps
- organizational apps
- private packages

The runtime should support package installation independently of the official catalog.

The catalog protocol and package model should be open.

The hosted catalog service may be proprietary.

---

# 14. Application Categories

## 14.1 Portable Applications

Depend only on capabilities available across multiple environments.

These are candidates for movement between local and hosted runtimes.

---

## 14.2 Device Applications

Depend on machine-specific functionality such as:

- local filesystem
- clipboard
- native applications
- local processes
- hardware
- private network resources

These may only run on compatible runtimes.

---

## 14.3 Hosted Applications

Designed primarily for server or cloud execution.

Examples include:

- internal dashboards
- APIs
- scheduled services
- team tools
- public applications

---

## 14.4 Hybrid Applications

Combine one or more hosted components with connected device runtimes.

They may use:

- hosted UI
- central persistence
- cloud scheduling
- local filesystem access
- machine automation
- private network access

The architecture should allow this model even if it is not initially implemented.

---

# 15. User Modes

## 15.1 Standalone Local User

Uses one runtime with no account.

Must receive a complete product experience.

---

## 15.2 Self-Hosted Standalone User

Runs the open-source runtime on personal or company infrastructure without using the commercial cloud.

This is a supported use case.

---

## 15.3 Multi-Device Individual

Connects multiple runtimes to a cloud workspace for centralized visibility.

---

## 15.4 Mixed Local and Hosted User

Runs some applications locally and others remotely while managing them together.

---

## 15.5 Self-Hosted Team

Uses organization-owned runtimes while purchasing cloud coordination and management.

---

## 15.6 Managed Cloud Team

Uses platform-operated application hosting and cloud orchestration.

---

## 15.7 Hybrid Organization

Uses a mixture of:

- employee devices
- company servers
- third-party infrastructure
- managed hosting
- public applications
- private applications

under one workspace.

---

# 16. Functional Requirements

## 16.1 Standalone Runtime

Users must be able to:

- install the runtime
- use it without an account
- install apps
- remove apps
- enable and disable apps
- configure apps
- view logs
- inspect health
- manage schedules
- manage permissions
- access application interfaces
- survive device restarts
- operate offline

---

## 16.2 Application Management

The system must support:

- identity
- versions
- lifecycle
- capabilities
- configuration
- permissions
- compatibility
- deployment constraints
- health

---

## 16.3 Cloud Connectivity

A standalone runtime must optionally be connectable to a workspace.

Connecting must preserve:

- installed apps
- local configuration
- local data
- local secrets
- local functionality

Disconnecting must not inherently uninstall or disable local applications.

---

## 16.4 Workspace Management

The commercial service must support:

- workspaces
- users
- teams
- runtimes
- environments
- applications
- deployments
- access
- policy
- health visibility

---

## 16.5 Deployment

Compatible applications should be deployable to supported environments.

Users should be able to understand:

- available targets
- compatibility
- deployment status
- version
- health
- accessibility

---

## 16.6 Multiple Deployments

The same application may have multiple deployments.

Each deployment may independently define:

- target environment
- configuration
- secrets
- access
- version
- lifecycle state

---

## 16.7 Managed Hosting

Compatible applications may be deployed into infrastructure operated by the commercial service.

Users should not need to manage the underlying infrastructure.

---

## 16.8 Self-Hosted Environments

Users must be able to operate applications on runtimes they control.

Cloud participation should remain optional.

---

## 16.9 Cloud-Orchestrated Local Applications

The cloud may coordinate:

- installation intent
- desired version
- enablement
- deployment configuration
- update policy
- organizational policy

The runtime should continue actual execution independently.

---

## 16.10 Control-Plane Independence

The open-source runtime should not require the official control plane.

Alternative controllers should remain technically possible through the public protocol.

The official cloud is not required to provide compatibility guarantees for arbitrary third-party controllers beyond the published protocol.

---

# 17. Desired-State Model

The commercial cloud should primarily operate through desired state.

The cloud expresses intent.

The runtime reports reality.

Desired state may include:

- application should be installed
- application should run a given version
- application should be enabled
- deployment configuration should have a particular value
- application should be removed

Observed state may include:

- installed
- running
- stopped
- unhealthy
- outdated
- disconnected
- incompatible

A loss of cloud connectivity must not automatically alter functioning local applications.

---

# 18. Data Ownership Model

The system should distinguish among:

## Application Data

Data created and consumed by an application.

## Application Configuration

User-controlled application behavior.

## Runtime State

Information needed for the runtime itself.

## Deployment State

Information related to one application instance.

## Workspace State

Organizational and orchestration information owned by the cloud control plane.

## Observability Data

Logs, events, metrics, and health information.

Connecting a runtime to the cloud should not automatically transfer application data or secrets into cloud storage.

Data movement should be explicit.

---

# 19. Trust Model

The system should distinguish among:

- trusted local applications
- sandboxed applications
- organization-approved applications
- third-party catalog applications

Permissions should correspond to actual enforceable boundaries.

The system should not imply isolation that does not exist.

---

# 20. Non-Functional Requirements

## 20.1 Reliability

- one app failure must not crash unrelated apps
- one app failure must not crash the runtime
- cloud failure must not break independent local execution
- network loss must not disable local apps
- runtime restart should restore expected operation

---

## 20.2 Simplicity

Users should primarily think in terms of:

- apps
- runtimes
- environments
- deployments
- capabilities
- workspaces

Infrastructure internals should remain secondary.

---

## 20.3 Portability

Applications should avoid unnecessary dependence on environment-specific infrastructure.

Compatible apps should move between supported environments without major rearchitecture.

---

## 20.4 Management Portability

The runtime should remain usable without the official commercial control plane.

The runtime should not encode proprietary cloud assumptions into its fundamental application model.

---

## 20.5 Transparency

Users should be able to determine:

- where an app runs
- where data lives
- where secrets live
- who manages it
- who can access it
- which cloud dependencies exist
- which capabilities prevent deployment elsewhere

---

## 20.6 Security

The system must support appropriate isolation between:

- applications
- runtimes
- users
- capabilities
- environments

Cloud connectivity must not unnecessarily expand access to local applications.

---

## 20.7 Privacy

Local-only applications should not send application activity to the commercial service.

Cloud-connected runtimes should clearly distinguish:

- orchestration metadata
- application data
- logs
- configuration
- secrets

---

## 20.8 Offline Operation

Standalone operation must work offline.

Cloud-managed runtimes should degrade gracefully when disconnected.

---

## 20.9 Performance

The runtime should remain lightweight enough for many small applications to coexist without excessive baseline resource consumption.

---

## 20.10 Scalability

The architecture should support growth from:

- one runtime with a few apps

to:

- organizations with many users, runtimes, environments, and deployments

without replacing the core application model.

---

## 20.11 Extensibility

New:

- capabilities
- execution models
- deployment providers
- control planes
- environments

should be introducible without unnecessary application model changes.

---

## 20.12 Recoverability

The system should tolerate:

- device restart
- runtime restart
- application crash
- control-plane outage
- network interruption

and restore expected state where possible.

---

## 20.13 Clean Removal

Removing an app should not leave unmanaged processes, routes, services, or hidden dependencies.

Leaving the commercial cloud should not leave local applications in an unusable state.

---

# 21. User Experience Requirements

## 21.1 Local Experience Must Be Complete

A standalone user should see a complete application platform, not an advertisement for the cloud product.

Core local experience should include:

- apps
- status
- logs
- schedules
- configuration
- permissions
- installation
- updates

Cloud connection should be presented as an optional extension.

---

## 21.2 Cloud Experience Should Extend the Same Mental Model

The cloud should use the same core concepts as the local runtime.

The user should not need to learn a fundamentally different system.

---

## 21.3 Runtime Location Must Be Visible

Every deployment should clearly indicate where it runs.

Examples:

- this device
- another device
- home server
- company server
- staging
- production
- managed cloud

---

## 21.4 Local-to-Hosted Progression Should Be Natural

A useful local application should be able to become a hosted or shared application without being recreated from scratch.

---

## 21.5 Compatibility Must Be Explicit

If an application cannot run in a particular environment, the system should explain which capability prevents it.

---

## 21.6 Commercial Features Should Feel Additive

Users should perceive the commercial service as adding:

- shared management
- deployment
- observability
- hosting
- teams
- access
- convenience

rather than unlocking artificially withheld runtime functionality.

---

# 22. Product and Commercial Model

The architecture should naturally support several offerings.

## Open-Source Runtime

Self-operated local or server execution.

Expected to be fully functional.

---

## Cloud Workspace

Central coordination for users with multiple runtimes or environments.

Commercial subscription.

---

## Bring-Your-Own Infrastructure

Customer-owned execution combined with commercial cloud management.

Commercial subscription.

---

## Managed Hosting

Platform-operated infrastructure.

Commercial subscription and potentially usage-based pricing.

---

## Team and Organization Features

Commercial functionality around:

- users
- roles
- sharing
- organizational policy
- centralized access
- fleet management

---

## Enterprise Self-Hosted Control Plane

Potential future offering if market demand justifies it.

This is not required for the initial open-source commitment.

---

# 23. High-Level Product Evolution

## Phase 1 — Open-Source Local Runtime

Build an excellent standalone runtime capable of replacing duplicated infrastructure across existing tools.

Primary goal:

> One runtime replaces the daemon, routing, scheduling, logging, configuration, and notification infrastructure otherwise duplicated across small applications.

---

## Phase 2 — Public Control Protocol and Cloud Workspace

Allow runtimes to optionally connect to the commercial service.

Primary goal:

> Multiple independent runtimes can be coordinated without giving up local autonomy.

---

## Phase 3 — Remote and Self-Hosted Environments

Allow runtimes on servers and private infrastructure to participate in workspaces.

Primary goal:

> Local and remote applications share one operating model.

---

## Phase 4 — Portable Deployment Providers

Expand support for applications that can move among compatible execution environments.

Primary goal:

> The application contract remains stable while deployment location changes.

---

## Phase 5 — Managed Hosting

Provide first-party hosted execution.

Primary goal:

> A personal application can become a reliable hosted internal tool with minimal operational work.

---

## Phase 6 — Team and Organization Platform

Add collaboration, access management, fleet governance, policies, and organizational workflows.

Primary goal:

> Support the long tail of applications and automations across an organization.

---

# 24. Architectural Invariants

The following should be treated as long-term architectural rules.

1. A runtime must remain independently useful.
2. A cloud account must not be required for local operation.
3. Cloud disconnection must not inherently break local applications.
4. Everything required to operate one runtime should remain open source.
5. The runtime owns execution.
6. The cloud owns coordination and orchestration.
7. The control-plane protocol should be public.
8. Runtimes should not be technically restricted to the official cloud.
9. Applications should declare capabilities rather than vendor-specific infrastructure wherever practical.
10. Deployment location, management location, and accessibility are independent concepts.
11. Applications should remain isolated from the runtime and one another as appropriate.
12. The same application model should span local, self-hosted, and managed environments.
13. Applications that are not portable should be represented honestly.
14. Deployment provider interfaces should be extensible.
15. Local observability and administration should not require the cloud.
16. User configuration and application data should remain portable wherever practical.
17. Managed hosting should optimize for the platform application model rather than arbitrary infrastructure.
18. Commercial value should come primarily from coordination, collaboration, hosting, and operated services.
19. The local UI should remain a first-class product.
20. The commercial cloud should enhance the runtime rather than become its dependency.

---

# 25. Success Criteria

The platform is successful if all of the following are true.

## Local Simplicity

A user can run multiple applications on one device without independently managing their infrastructure.

## Local Independence

They can continue doing so indefinitely without the commercial service.

## Open Ecosystem

Developers can build apps, runtimes, deployment providers, and compatible tooling using public platform contracts.

## Unified Management

Users who adopt the cloud can manage local, self-hosted, and managed deployments from one workspace.

## Consistent Application Model

Developers do not need to create fundamentally different applications for local and hosted environments.

## Graceful Portability

Applications using portable capabilities can move between compatible execution targets with minimal changes.

## Honest Compatibility

Device-specific applications remain supported without being artificially forced into a hosted model.

## Easy Progression

A personal application can naturally become:

- multi-device
- self-hosted
- team-accessible
- cloud-hosted
- organizational

without abandoning its original application model.

## Cloud Optionality

A cloud-connected local runtime can leave the commercial service without losing its core local applications.

## Commercial Value

Users are willing to pay because the commercial service materially reduces coordination and operational burden, not because fundamental runtime capabilities were withheld.

## Operational Simplicity

The platform removes substantially more operational complexity than it introduces.

---

# 26. Guiding Product Principle

The defining boundary of the platform should remain:

> **The runtime is software the user owns. The cloud is a service the user can choose.**

The open-source runtime should be enough to build something valuable.

The commercial cloud should make that runtime dramatically more useful when users need multiple devices, teams, shared applications, deployment orchestration, or managed infrastructure.
