# Paraco Open-Source Runtime and Application Framework

## Repository Scope

Paraco is an open-source runtime and application framework for creating,
installing, running, and managing small applications on local machines and
self-hosted infrastructure. A standalone runtime requires no account or subscription.

This specification describes the intended open-source design, including future
work; it is not a claim that every feature is implemented. The initial scope is
one machine. Public control protocols, the optional connector, runtime-side
reconciliation, and deployment provider interfaces are deferred until the
standalone foundation works. See README.md and TODO.md for implementation status.

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

## Purpose and Goals

Provide reusable process management, routing, configuration, persistence,
scheduling, secrets, logging, health monitoring, notifications, updates,
permissions, and deployment contracts for small applications.

Supported use cases include personal productivity tools, internal tools,
automations, dashboards, scheduled jobs, AI-generated utilities, small web
applications, lightweight services, and device-connected tools.

Users should be able to install and remove applications easily, operate offline,
and move compatible applications between local and self-hosted environments.
The framework should be straightforward for developers and AI coding agents to target.

## Non-Goals

Paraco does not replace Kubernetes, arbitrary container orchestration,
general-purpose infrastructure management, virtual machines, enterprise endpoint
management, unrestricted PaaS platforms, desktop application frameworks, IDEs,
configuration management systems, or infrastructure-as-code platforms.

## Open-Source Runtime and Framework

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
- optional control-plane connector client
- desired-state reconciliation logic
- deployment provider interfaces
- public control-plane protocol definitions

Users should be able to fork the project and operate it independently.

## Open Protocols

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

## Deployment Provider Extensibility

The mechanism used to support deployment targets should be extensible.

The open-source framework should define the interface by which deployment environments participate in the system.

This should allow support for:

- local machines
- Linux servers
- private infrastructure
- third-party cloud providers
- community deployment targets

## Design Principles

A runtime is independently useful and owns application execution. Everything
required to create, install, run, and manage applications on one runtime is open
source. Local administration and observability remain fully functional offline.

Execution location, management location, and accessibility are independent.
Users must understand where applications, configuration, data, and secrets live,
who manages them, and who can access them. Data movement must be explicit.

An optional controller must not become an execution dependency. Disconnection
must not uninstall, disable, or change functioning independent applications.
Alternative controllers must be possible through public protocols.

## Portable Application Model

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

## Execution Is Independent of Accessibility

The platform must distinguish among:

- where an application executes
- who manages it
- who can access it

An application may be:

- device-only
- private
- network-accessible
- authenticated over the internet
- public

These are independent dimensions.

## Architecture

The open-source architecture consists of the application model, runtime,
capability layer, public control protocol, and extensible execution and deployment
interfaces. Runtimes may operate on laptops, desktops, workstations, home servers,
private servers, company infrastructure, and compatible third-party infrastructure.

## Application Model

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

## Control Protocol

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

## Runtime Core

Responsible for:

- application execution
- lifecycle management
- capability delivery
- local state
- health
- runtime diagnostics
- local control

This module is open source.

## Application Manager

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

## Application Execution Layer

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

## Web Gateway

Provides consistent application web access.

Responsibilities include:

- routing
- stable application addresses
- application isolation
- local access
- environment-aware access

The local implementation is open source.

## Capability Layer

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

### AI Proxy Capability

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
must work independently. Calls to external AI providers
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

## Configuration and Settings

The system should distinguish among:

### Application Configuration

Behavior inherent to the application.

### Deployment Configuration

Behavior specific to one deployed instance.

### Runtime Configuration

Behavior of a specific runtime.

## Local Control Interface

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

## Optional Control-Plane Connector

An optional runtime component connects a runtime to a control plane.

It should support:

- runtime registration
- secure communication
- state synchronization
- desired-state receipt
- observed-state reporting

The connector and runtime-side reconciliation logic should be open source.

## Desired-State Reconciler

The runtime should be capable of receiving desired state from a control plane and reconciling its actual state toward that intent.

Examples include:

- application should exist
- application should be enabled
- application should run a particular version
- configuration should match intended state
- application should be removed

The reconciler runs locally and is open source.

## Deployment Providers

Deployment providers represent environments capable of hosting applications.

Potential categories include:

- local runtime
- self-hosted runtime
- third-party cloud
- specialized execution environment

The provider interface should be open and extensible.

Community providers should be possible.

## Self-Hosted Infrastructure

Users must be able to operate applications on infrastructure they control,
including private servers, company infrastructure, and supported external platforms.

## Authentication and Access

Authentication is separate from application execution. Applications may be
local-only, private, network-accessible, externally authenticated, or public.
Local applications require no external authentication service.

## Secrets Management

The runtime supports local secrets scoped to the runtime, application deployment,
or environment. Capability requests are distinct from user grants.

## Observability

Each runtime should provide local observability.

This includes:

- application health
- logs
- activity
- scheduled executions
- failures
- runtime status

## Application Catalog

The platform may provide support for discovering installable applications.

The ecosystem may contain:

- first-party apps
- community apps
- organizational apps
- private packages

The runtime should support package installation independently of the official catalog.

The catalog protocol and package model should be open.

## Application Categories

### Portable Applications

Depend only on capabilities available across multiple environments.

These are candidates for movement between local and hosted runtimes.

### Device Applications

Depend on machine-specific functionality such as:

- local filesystem
- clipboard
- native applications
- local processes
- hardware
- private network resources

These may only run on compatible runtimes.

### Hosted Applications

Designed primarily for server or cloud execution.

Examples include:

- internal dashboards
- APIs
- scheduled services
- team tools
- public applications

### Hybrid Applications

Combine one or more hosted components with connected device runtimes.

They may use:

- hosted UI
- central persistence
- remote scheduling
- local filesystem access
- machine automation
- private network access

The architecture should allow this model even if it is not initially implemented.

## User Modes

Standalone local users operate one runtime without an account and receive a
complete application platform. Self-hosted users run the runtime on personal
or company infrastructure with the same application model.

## Functional Requirements

### Standalone Runtime

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

### Application Management

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

### Deployment

Compatible applications should be deployable to supported environments.

Users should be able to understand:

- available targets
- compatibility
- deployment status
- version
- health
- accessibility

### Multiple Deployments

The same application may have multiple deployments.

Each deployment may independently define:

- target environment
- configuration
- secrets
- access
- version
- lifecycle state

## Control-Plane Independence

The runtime does not require any particular controller. Runtime identity,
registration, capabilities, observed and desired state, deployment intent,
configuration references, health, versions, and application inventory are public
contracts. The runtime remains locally operable when a controller is unavailable.

## Desired-State Model

Optional controllers express intent through desired state.

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

## Data Ownership Model

The system should distinguish among:

### Application Data

Data created and consumed by an application.

### Application Configuration

User-controlled application behavior.

### Runtime State

Information needed for the runtime itself.

### Deployment State

Information related to one application instance.

### Observability Data

Logs, events, metrics, and health information.

Connecting a runtime to a controller must not automatically transfer application data or secrets. Data movement must be explicit.

## Trust Model

The system should distinguish among:

- trusted local applications
- sandboxed applications
- organization-approved applications
- third-party catalog applications

Permissions should correspond to actual enforceable boundaries.

The system should not imply isolation that does not exist.

## Non-Functional Requirements

### Reliability

- one app failure must not crash unrelated apps
- one app failure must not crash the runtime
- controller failure must not break independent local execution
- network loss must not disable local apps
- runtime restart should restore expected operation

### Simplicity

Users should primarily think in terms of:

- apps
- runtimes
- environments
- deployments
- capabilities

Infrastructure internals should remain secondary.

### Portability

Applications should avoid unnecessary dependence on environment-specific infrastructure.

Compatible apps should move between supported environments without major rearchitecture.

### Management Portability

The runtime should remain usable without an external control plane.

The application model should remain independent of controller implementations.

### Transparency

Users should be able to determine:

- where an app runs
- where data lives
- where secrets live
- who manages it
- who can access it
- which external dependencies exist
- which capabilities prevent deployment elsewhere

### Security

The system must support appropriate isolation between:

- applications
- runtimes
- users
- capabilities
- environments

Optional controller connectivity must not unnecessarily expand access to local applications.

### Privacy

Standalone applications must not send application activity to an external
management service. Optional connections must distinguish orchestration metadata,
application data, logs, configuration, and secrets; transfers require explicit intent.

### Offline Operation

Standalone operation must work offline. Capabilities inherently requiring a
network connection should fail clearly without disabling unrelated applications.

### Performance

The runtime should remain lightweight enough for many small applications to coexist without excessive baseline resource consumption.

### Scalability

The architecture should support growth from:

- one runtime with a few apps

to:

- organizations with many users, runtimes, environments, and deployments

without replacing the core application model.

### Extensibility

New:

- capabilities
- execution models
- deployment providers
- control planes
- environments

should be introducible without unnecessary application model changes.

### Recoverability

The system should tolerate:

- device restart
- runtime restart
- application crash
- control-plane outage
- network interruption

and restore expected state where possible.

### Clean Removal

Removing an app should not leave unmanaged processes, routes, services, or hidden dependencies.

Disconnecting a controller must leave independent local applications usable.

## User Experience Requirements

### Local Experience Must Be Complete

A standalone user should have a complete application platform.

Core local experience should include:

- apps
- status
- logs
- schedules
- configuration
- permissions
- installation
- updates

Controller connections should be presented as optional extensions.

### Runtime Location Must Be Visible

Every deployment should clearly indicate where it runs.

Examples:

- this device
- another device
- home server
- company server
- staging
- production

### Local-to-Hosted Progression Should Be Natural

A useful local application should be able to become a hosted or shared application without being recreated from scratch.

### Compatibility Must Be Explicit

If an application cannot run in a particular environment, the system should explain which capability prevents it.

## Open-Source Evolution

1. Build a complete standalone runtime, local control interface, and capabilities.
2. Publish control protocols and add an optional connector and local reconciler.
3. Support remote, self-hosted execution environments.
4. Expand portable deployment provider interfaces and community implementations.

## Architectural Invariants and Success Criteria

- One runtime removes infrastructure duplicated across small applications.
- Local operation and administration remain complete and independently useful.
- The runtime owns execution and must survive application failures.
- Applications declare capabilities instead of vendor-specific infrastructure.
- Device-specific applications remain supported with honest compatibility constraints.
- Isolation claims reflect enforceable execution boundaries.
- Public contracts support applications, runtimes, providers, and alternative tooling.
- Configuration and application data remain portable wherever practical.
- Compatible applications move between environments without fundamental rewrites.
- Deployment location, management, and access remain independent concepts.
- Removal leaves no unmanaged processes, routes, services, or hidden dependencies.
- The platform removes more operational complexity than it introduces.
