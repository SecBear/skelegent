# Executive Summary

In 2025-2026, the architecture for agentic AI tool execution has standardized around a spectrum of isolation technologies, reflecting a mature understanding of the associated security risks. For executing untrusted, LLM-generated code, production systems are increasingly favoring strong isolation boundaries like microVMs (e.g., Firecracker, Kata) or user-space kernels (e.g., gVisor) over traditional containers. This shift is driven by the need to mitigate kernel-exploit and container-escape vulnerabilities. The primary security threats remain prompt-injection attacks leading to data exfiltration and the execution of malicious code from third-party tool marketplaces (skills/plugins). In response, production systems gate tool calls with a multi-layered defense: capability-based controls requiring explicit user approvals for risky actions, the injection of short-lived and least-privileged credentials at runtime, and the enforcement of structured I/O to prevent data leakage. Concurrency and parallelism are achieved through architectural patterns that fan-out tool calls across numerous isolated, fast-booting sandboxes, managed with per-tool concurrency caps and idempotency controls to meet latency SLOs. Emerging areas of focus include managing supply-chain risks from the tool ecosystem, robust result sanitization via schemas, strict lifecycle management for ephemeral sandboxes to prevent state poisoning, and enhanced observability with action-level risk grading and comprehensive audit trails.

# Tool Execution Architecture Patterns

The latest architectural patterns for tool execution in LLM agents are centered on robust isolation, granular security controls within the agent's control loop, and secure dependency management. A dominant pattern is the remote execution of tools in sandboxed environments, treating the agent runtime itself as untrusted code.

**Isolation Technologies and Sandboxing:**
- **MicroVMs (Firecracker/Kata):** Considered the strongest isolation boundary for untrusted code. They provide a dedicated kernel, mitigating container-escape risks. Platforms like E2B and Fly.io leverage Firecracker microVMs to offer on-demand, fast-booting, isolated Linux environments for agents. Their sub-second start times make them ideal for ephemeral, parallel tool execution.
- **User-Space Kernels (gVisor):** These offer a compromise between the security of full VMs and the performance of containers by mediating syscalls to reduce host kernel exposure. Claude Code's cloud sandboxes have been updated to provide 'gVisor-class isolation'.
- **Containers (Docker):** Still widely used for their compatibility and speed, as seen in OpenHands which uses Docker for its default runtime. However, security guidance emphasizes that they must be paired with strict egress filtering, filesystem scoping, and resource limits to be effective. Modal also provides secure containers for running untrusted code at runtime.
- **Runtime Isolates (WASM/V8):** These offer the lowest latency and are best suited for narrowly scoped, stateless functions, such as proxying tool calls or running safe logic at the edge. They are less common for general-purpose agentic workflows that require full POSIX execution environments.

**Agent Control Loop Design and Security Checks:**
- **Full-Surface Sandboxing:** A key principle, articulated by NVIDIA, is to sandbox the *entire* agentic workflow, not just the final tool invocation. This includes IDE hooks, startup scripts, and skill installations, to close off indirect execution paths.
- **Approval Mechanisms:** Production systems have moved beyond simple on/off approvals. OpenAI's Agent Builder uses a 'human approval node' for explicit confirmation. OpenHands implements a more dynamic `SecurityAnalyzer` that rates the risk of each action and a `ConfirmationPolicy` that can auto-approve low-risk calls while pausing for user confirmation on high-risk ones. Claude Code uses a perimeter-based model, auto-approving calls to predefined domains and repositories while blocking and logging unauthorized attempts.
- **Credential and Secret Injection:** The standard practice is to avoid embedding long-lived credentials. Instead, short-lived, least-privileged tokens are injected at call time from a secrets manager. OpenHands provides automatic secret detection and injection as environment variables, along with masking secret values in logs and the LLM context. Claude Code takes this a step further by using a managed proxy for Git access, ensuring user tokens never enter the sandbox environment.
- **Structured I/O and Result Sanitization:** To combat prompt injection and data exfiltration between tools, platforms like OpenAI advocate for using structured outputs (e.g., JSON schemas, enums) to eliminate freeform data channels. This is often combined with guardrails that sanitize inputs by redacting PII and detecting jailbreak attempts.

# Dominant Security Threats

## Threat Type

Malicious Tool Supply Chain

## Description

This threat manifests when an attacker develops a malicious tool or 'skill' and uploads it to a public tool platform or registry. When a user's agent, seeking to accomplish a task, discovers and installs this seemingly legitimate tool, it is executed within the user's environment. This execution can compromise the user's security and privacy by exfiltrating data, stealing credentials, or establishing persistence on the system. This is a significant supply-chain risk for the agent ecosystem.

## Example Vector

An attacker creates and uploads a malicious tool to a tool platform. A user's agent, tasked with a specific goal, installs this tool. Upon execution, the tool compromises the user's device or cloud environment. The 'MalTool' paper details a framework that automatically generates such malicious tools, demonstrating that existing text-based and even some code-based vetting techniques can fail to reliably detect these attacks, necessitating runtime defenses like sandboxing and dynamic analysis.


# Isolation Technologies Comparison

## Technology Category

MicroVMs

## Example Implementation

Firecracker/Kata Containers

## Isolation Boundary

Hardware Virtualization (Dedicated Kernel)

## Security Strength

Strongest tenancy boundary for untrusted LLM-generated code. The use of a dedicated kernel for each microVM mitigates kernel-level exploits and container-escape vectors, providing a robust security posture recommended for high-risk workloads.


# Micro Vm Sandboxing

MicroVM technology, particularly AWS's Firecracker, represents the gold standard for securely executing untrusted code from AI agents. It provides strong, hardware-enforced isolation by giving each sandbox its own dedicated, minimal kernel. This architecture fundamentally mitigates the risk of container-escape vulnerabilities and other kernel-level exploits that can affect shared-kernel technologies. Despite this high level of security, microVMs like Firecracker are optimized for extremely fast boot times, often starting in under a second, which makes them practical for the ephemeral, on-demand nature of agent tool execution. This combination of security and performance has led to its adoption by leading 'sandbox-as-a-service' platforms, including E2B, which provides on-demand Firecracker-powered Linux VMs via an SDK, and Fly.io, which uses Firecracker to power its fast-launching 'Machines' for scalable agent backends. The technology is also foundational to serverless platforms like AWS Lambda.

# Application Kernel Sandboxing

Application kernel technology, exemplified by Google's gVisor, offers a compelling middle ground between the performance of containers and the security of full virtual machines. It functions as a userspace kernel that intercepts and handles system calls made by the sandboxed application, acting as a proxy to the host kernel. This mediation layer significantly reduces the host kernel's attack surface, providing a strong security boundary against exploits without requiring hardware virtualization. This approach is increasingly adopted for code-execution sandboxes where density and lower overhead are important considerations. A prominent example is Anthropic's Claude Code, which leverages 'gVisor-class isolation' for its cloud sandboxes. This allows it to securely run agent-generated code while enforcing strict allowlists for filesystem and network access, achieving a high degree of safety without the performance penalty of a full VM.

# Container And Isolate Sandboxing

While stronger isolation methods like microVMs are preferred for untrusted code, traditional containers and lightweight isolates continue to play a significant role in agent architectures due to their performance and compatibility. Traditional containers, such as those managed by Docker, are used by platforms like OpenHands to provide filesystem isolation, network policy enforcement, and resource limits. This approach is valued for its speed and broad compatibility, but because it relies on a shared host kernel, it is considered less secure and must be paired with strict egress filtering and filesystem scoping to be effective. At the other end of the spectrum are lightweight isolates like WebAssembly (WASM) and V8 isolates, used by platforms like Cloudflare Workers and Deno Deploy. These offer the best performance and lowest latency by running code within a sandboxed language runtime. Their primary use case is for narrowly scoped, often stateless functions, such as proxying tool calls or performing simple data transformations at the edge. They are less suitable for running arbitrary agent code that requires a full POSIX-compliant execution environment.

# Credential Management And Injection

Production systems are moving towards a robust, multi-layered approach for securely handling credentials. The core principle is to use dedicated, non-privileged, and short-lived tokens with narrowly defined scopes (e.g., limited OAuth consent) to minimize the blast radius of a compromise. Instead of embedding long-lived credentials within agent code or sandbox environments, secrets are injected at call time, often as environment variables, via a secrets manager. Platforms like OpenHands provide automatic secret management, which includes detecting secrets in commands, injecting them into the sandboxed execution, and critically, masking these secret values in logs and the LLM context to prevent accidental leakage. An even more secure pattern involves brokered access, such as the proxy-mediated Git access used by Claude Code, which keeps raw tokens and credentials completely off the sandbox, further reducing the attack surface.

# Result Sanitization And Policy Enforcement

Securing the data flow to and from tools involves a combination of input sanitization, output structuring, and policy enforcement. For incoming data, systems use guardrails to sanitize inputs by redacting Personally Identifiable Information (PII) and detecting jailbreak attempts. To secure the output and inter-tool communication, the dominant method is to enforce structured outputs. By using enums, JSON schemas, and required field names, developers can eliminate freeform text channels that attackers could exploit for indirect prompt injection and data exfiltration. This is complemented by strict policy enforcement at the platform level. Network egress controls, implemented as allow-lists, block access to all but pre-approved domains and IPs, preventing data leakage. Similarly, filesystem scoping prevents the agent from reading or writing files outside of its designated workspace, with specific rules blocking writes to configuration files or agent extensions to prevent attackers from establishing persistence.

# Capability Based Security And Approvals

Modern agentic systems manage permissions through a combination of capability-based models and human-in-the-loop approvals. The foundational approach is capability-scoped tool access, where agents are granted the minimum permissions necessary. For high-risk actions, human confirmation is critical; platforms like OpenAI's Agent Builder explicitly recommend using a 'human approval node' so end-users can review and confirm every operation. To combat 'approval fatigue,' systems like OpenHands are introducing more sophisticated mechanisms. They use an LLM-assisted 'SecurityAnalyzer' to rate the risk of each tool call (e.g., low, medium, high) and a 'ConfirmationPolicy' to determine whether to auto-approve the action or pause for user confirmation. This is coupled with a default-deny security posture for network and filesystem access outside a defined workspace. Furthermore, capability management extends to the supply chain; security guidance recommends treating the installation of a new 'skill' or tool from a marketplace as a privileged action equivalent to executing third-party code, requiring explicit user approval and runtime monitoring.

# Concurrency And Parallelism Strategies

Modern agentic workflows employ several strategies to manage concurrent and parallel tool execution, focusing on scalability, latency, and security. These patterns are heavily reliant on cloud-native architectures and fast, ephemeral execution environments.

**Workflow Orchestration and Fan-Out Execution:**
The primary pattern for parallelism is 'fan-out', where a central agent controller dispatches multiple tool calls to run simultaneously across a fleet of isolated sandboxes. This is managed using:
- **Per-Tool Concurrency Caps:** Limiting the number of simultaneous calls to a specific tool or API endpoint to avoid rate limiting and service degradation.
- **Circuit Breakers:** Automatically halting calls to a tool that is consistently failing or timing out.
- **Idempotency and Retries:** Ensuring that tool calls can be safely retried without causing duplicate side effects, typically by using idempotency keys. This, combined with hedging (sending the same request to multiple instances and using the first response), helps tame flaky downstream providers.

**Leveraging Fast-Boot Execution Substrates:**
Horizontal scaling for parallel tool execution is enabled by platforms providing fast-booting virtual machines or containers. Fly.io Machines, which are Firecracker-backed VMs, are a prime example. Their key features supporting this pattern include:
- **Sub-Second Start Times:** Machines can be created and started in 'well under a second', making it feasible to spin up a new, clean environment for each parallel task without significant latency overhead.
- **API-Driven Lifecycle:** A simple REST API allows for programmatic control to create, start, stop, and destroy machines, enabling dynamic scaling based on workload.
- **Regional Placement and Cloning:** Machines can be placed in any region, close to users or data sources to reduce latency. They can also be cloned to quickly replicate environments for parallel processing across different geographies.
- **Scale-to-Zero:** The ability to quickly stop idle machines and start them again on demand allows for highly efficient, cost-effective resource utilization, as compute is only consumed during active tool execution.

**Maintaining Security and State Consistency:**
- **Isolation:** Security across concurrent executions is maintained by the fundamental principle of running each tool call in its own strongly isolated sandbox (microVM or container). This prevents tasks from interfering with each other's filesystems, networks, or processes.
- **State Management:** State consistency is often handled by designing tasks to be stateless. Fast-boot VMs can be configured with ephemeral storage, providing a 'blank slate on every startup'. For tasks requiring persistence, dedicated volumes can be attached to the sandbox.
- **Non-Blocking Approvals:** A key challenge is that human-in-the-loop approvals can serialize an entire workflow, negating the benefits of parallelism. Systems like OpenHands address this by using a risk-based `ConfirmationPolicy`. This allows many low-risk, concurrent operations to proceed automatically while only pausing the specific high-risk actions that require user confirmation, thus preserving overall workflow concurrency.

# Claude Code Architecture Analysis

## Isolation Technology

Cloud sandboxes for Claude Code utilize gVisor-class isolation. This approach involves mediating syscalls through a user-space kernel to reduce the host kernel's attack surface, offering a strong security boundary with lower overhead than full hardware virtualization.

## Credential Handling

The system employs a brokered access model for credentials. Specifically, all Git operations are routed through an Anthropic-managed proxy. This design ensures that user authentication tokens are never directly exposed to or stored within the sandboxed execution environment, minimizing the risk of credential leakage.

## Policy Enforcement

Security policy is enforced through a combination of strict, pre-defined allow-lists and a dynamic approval system. The platform uses allow-lists for both network access (approved domains) and filesystem access (approved repositories). For actions that fall within this pre-approved perimeter, the system grants automatic approval. However, any attempt to access unapproved network endpoints or filesystem locations is blocked, logged for audit, and triggers a prompt for user confirmation, thus reducing approval fatigue for common tasks while maintaining security guardrails.


# Openai Agent Security Practices

## Primary Sandbox

OpenAI's safety guidance for its Agent Builder emphasizes a layered approach to security rather than detailing a specific underlying sandbox technology like Firecracker or gVisor. The core of its sandboxing strategy involves process and data flow controls, such as requiring explicit user approvals for tool actions and structuring data to prevent injection attacks. The environment is designed to operate with guardrails that sanitize inputs and constrain outputs.

## Data Flow Control Method

The recommended technique to constrain data flow and mitigate prompt injection is the mandatory use of structured outputs between different nodes or tools in an agent's workflow. This involves defining strict schemas, using enums, and requiring specific field names. By eliminating freeform text channels between tools, the system reduces the attack surface for an attacker to exfiltrate private data or inject malicious commands into downstream tool calls.

## User Oversight Mechanism

The primary mechanism for ensuring user consent is the 'human approval node'. The official guidance mandates that tool approvals should always be enabled, which requires end-users to manually review and confirm every operation the agent proposes, especially reads and writes. This human-in-the-loop step acts as a critical checkpoint to prevent unintended or malicious actions.


# Openhands Architecture Analysis

## Risk Assessment Component

OpenHands incorporates a 'SecurityAnalyzer' component that leverages an LLM to perform risk assessment on each action the agent proposes. This component analyzes the tool call and assigns it a risk level, categorized as low, medium, high, or unknown. This automated risk scoring serves as the input for the subsequent approval policy.

## Approval Policy Component

The system uses a 'ConfirmationPolicy' component that works in conjunction with the SecurityAnalyzer. Based on the assessed risk level of a proposed action, this policy determines whether user approval is required before execution. This allows for a flexible workflow where low-risk actions can be auto-approved to reduce user friction, while high-risk operations are paused to await explicit user confirmation. The policy can be customized to fit different security postures.

## Sandboxing Technology

OpenHands provides first-class support for using Docker containers as its primary sandboxing technology. This approach isolates code execution, offering complete filesystem isolation, enforcement of network policies, and the ability to set resource limits (CPU, memory, disk). The architecture is also designed to be flexible, supporting execution in remote sandboxed environments to protect the client machine from untrusted code.

## Secrets Management Feature

The platform includes a built-in, automatic secrets management system. This feature is designed to detect secrets within bash commands, automatically inject them as environment variables into the sandboxed execution environment at runtime, and mask the secret values in all logs and the LLM context. This prevents sensitive credentials from being leaked or persisted in insecure locations.


# Devin And Proprietary Agents Analysis

The provided research materials do not contain specific details regarding the internal security architectures, isolation technologies, or sandboxing mechanisms used by highly autonomous and proprietary agents such as Devin, NanoClaw, and IronClaw. While the documents extensively cover security patterns and technologies used in open-source projects (OpenHands, OpenClaw) and platforms with developer documentation (Claude, OpenAI, E2B, Modal), these specific commercial agents are not detailed. The general industry trend points towards strong isolation like microVMs or gVisor for untrusted code, but there is no public information in the context to confirm if or how these specific agents implement such measures.

# Openclaw Security Guidance

## Official Risk Posture

Microsoft's official security guidance explicitly states that the OpenClaw self-hosted runtime should be treated as 'untrusted code execution with persistent credentials'. This assessment is based on the agent's behavior of ingesting untrusted text, downloading and executing third-party code ('skills'), and operating with the credentials assigned to it, which necessitates a high-security, containment-focused posture.

## Recommended Environment

Due to the high-risk nature of the runtime, the recommendation is that OpenClaw should be deployed 'only in a fully isolated environment'. The guidance specifies this means using a dedicated virtual machine or an entirely separate physical system. This isolation is intended to contain the blast radius in case of a compromise.

## Key Mitigations

The essential security guardrails for OpenClaw focus on containment and recoverability. Key mitigations include: 1) Using dedicated, non-privileged credentials with the least possible scope to limit potential damage. 2) Implementing continuous monitoring of the agent's activities. 3) Maintaining a rapid rebuild plan to quickly restore the environment to a known-good state after a security incident. 4) Treating the installation of any new 'skill' as a privileged event equivalent to executing third-party code, requiring careful vetting.


# Sandbox As A Service Platforms

## Platform Name

E2B

## Isolation Technology

Firecracker microVMs

## Persistence Model

Ephemeral

## Primary Use Case

AI agent tools and untrusted code execution. E2B provides on-demand, isolated Linux VMs via an SDK, designed specifically for agents to safely execute code, run tools, and process data.

## Platform Name

Modal

## Isolation Technology

Secure Containers

## Persistence Model

Ephemeral (Runtime-defined)

## Primary Use Case

ML/AI workloads and untrusted code execution. Modal allows for the dynamic creation of secure containers at runtime to run arbitrary code, such as executing code generated by an LLM or running tests against a git repository.

## Platform Name

Fly.io

## Isolation Technology

Firecracker microVMs

## Persistence Model

Configurable (Ephemeral by default, with optional persistent volumes)

## Primary Use Case

Horizontally scalable backends for AI agents. Fly.io Machines are fast-booting VMs with a REST API, enabling parallel, region-local, and ephemeral execution that can scale to zero, making it ideal for fanning out tool calls.

## Platform Name

Daytona

## Isolation Technology

Cloud-based sandboxes with process isolation

## Persistence Model

Stateful/Long-living

## Primary Use Case

Persistent development workspaces for AI agents. Daytona provides secure, isolated cloud environments designed to serve as remote runtimes for agent frameworks like OpenHands, where agents can edit files and execute commands.

## Platform Name

OpenHands

## Isolation Technology

Docker containers (default), with support for remote sandboxes

## Persistence Model

Configurable

## Primary Use Case

AI agent framework for software development. It uses Docker for local sandboxing but can integrate with remote environments like Daytona to protect client machines from untrusted code execution.

## Platform Name

Claude Code

## Isolation Technology

gVisor-class isolation

## Persistence Model

Stateful with a controlled perimeter

## Primary Use Case

Secure AI-powered coding assistant. It uses gVisor sandboxes with strict network/filesystem allowlists and a proxied Git connection to provide a safe execution environment that minimizes user approval prompts.


# Unaddressed Challenges In Tool Execution

Beyond basic isolation and credential management, production systems are grappling with several unaddressed challenges and newly emphasized security controls. A primary focus is on implementing egress policy as a first-class platform control, enforcing strict network allowlists and auditing violations, rather than leaving it to application logic. Another key area is 'full-surface sandboxing,' which extends isolation beyond just tool invocations to include all potential execution paths like IDE hooks, startup scripts, and skill installations, preventing attackers from finding unsandboxed bypasses. There is also a growing emphasis on lifecycle and ephemerality policies for sandboxes to prevent the accumulation of secrets, intellectual property, and poisoned state over time through scheduled rebuilds or ephemeral-per-task environments. Furthermore, the industry is recognizing the need for robust supply-chain governance for third-party skills and tools, treating their installation as a privileged code execution event that requires provenance checks, publisher allowlisting, and runtime monitoring. Other critical missing pieces include enforcing structured I/O boundaries with schema extraction to minimize prompt injection paths, operationalizing LLM-assisted action-level risk scoring to reduce approval fatigue while catching high-risk operations, and moving towards secrets brokering and proxy access (e.g., for Git) to keep raw credentials out of sandboxes entirely. Finally, a significant gap being addressed is the need for comprehensive observability and forensics, including persistent audit trails for all security-relevant actions like approvals, denials, and egress attempts.

# Emerging Security Standards

## Standard Or Initiative

MalTool and Taxonomy of Malicious Tool Behaviors

## Issuing Organization

Academic Researchers (as per arXiv:2602.12194)

## Focus Area

To define a threat taxonomy for malicious behaviors in third-party agent tools and to automatically generate malicious tools for evaluating the effectiveness of security defenses. The initiative highlights that existing text-based and static code analysis techniques are insufficient, advocating for new defenses centered on dynamic analysis and runtime monitoring within sandboxed environments.


# Incident Patterns And Cves

## Identifier

Malicious Tool Supply-Chain Attack

## Affected System

Agentic AI platforms and runtimes that utilize a tool ecosystem or marketplace, allowing users to install and execute third-party skills or tools (e.g., OpenClaw).

## Vulnerability Summary

An attacker develops a seemingly benign tool with hidden malicious behaviors and uploads it to a public tool platform. When a user's agent installs and executes the tool, it compromises the user's security and privacy. The vulnerability lies in the fact that existing defenses, such as text-based vetting and static code analysis by platform providers, fail to reliably detect these sophisticated, code-level malicious behaviors.

## Architectural Lesson

The incident highlights a fundamental architectural weakness: misplaced trust in the third-party tool supply chain. It demonstrates that treating tool installation as a low-risk event is a critical flaw. The key lesson is that agentic systems must adopt a zero-trust posture towards external tools, implementing robust runtime defenses like dynamic analysis within secure sandboxes and continuous monitoring, rather than relying solely on pre-distribution scanning and vetting.

