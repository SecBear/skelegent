# Executive Summary

The latest approaches to agentic AI system management in 2025-2026 emphasize robust control, observability, and resilience. For agent termination, the prevailing method is layered halting, which integrates agent self-assessment, programmatic output verification, progress and loop detection (e.g., checking semantic similarity of recent thoughts), and hard circuit breakers. These breakers enforce limits on iterations, time, cost, and consecutive errors, all managed within an explicit state machine (e.g., PLANNING, EXECUTING, TERMINATED). In observability, the industry is standardizing on OpenTelemetry (OTel), specifically its GenAI and emerging Agent semantic conventions. The best practice involves tracing an entire agent run as a root span, with LLM calls and tool executions as child spans, enriched with `gen_ai.*` attributes for token usage and `agent.*` attributes for decisions. Platforms like LangSmith and Datadog offer native support for these OTel conventions, with tools like LangSmith providing detailed cost attribution by mapping token usage to model pricing tables. Budget governance in production systems is implemented through a combination of engineering and product-level guardrails, including per-agent or per-task dollar caps, execution throttles, session time limits, and dynamic model routing to optimize costs. Enterprise governance further extends this with role-based access for agents, comprehensive audit trails, and integration with security platforms like SIEM/XDR. Finally, crash recovery is primarily addressed through durable execution frameworks. LangGraph offers built-in persistence and checkpointing, while Temporal is used as a powerful, general-purpose durable orchestration backbone. Both enable workflows to be paused and resumed after failures or human intervention, which necessitates careful design for idempotency to prevent duplicate side-effects upon replay.

# Key Trends 2025 2026

The agentic AI infrastructure landscape in 2025-2026 is defined by three major trends: protocol convergence, the operationalization of agent management, and the adoption of specialized tooling. 

First, there is a strong **convergence on open standards, particularly OpenTelemetry for observability**. The OpenTelemetry GenAI SIG is standardizing semantic conventions for AI agent applications and frameworks (including CrewAI, AutoGen, and LangGraph), creating a vendor-neutral way to instrument and trace agent behavior. This move is solidifying OpenTelemetry as the 'de facto standard for agent tracing,' with broad adoption by commercial platforms like Datadog and open-source tools like Arize AI's Phoenix.

Second is the **rise of 'AgentOps'**, which applies DevOps principles to the entire lifecycle of AI agents. This trend involves professionalizing agent management by establishing agent registries, implementing robust telemetry, defining lifecycle and rollback procedures, and integrating agent logs into security platforms like SIEM/XDR. A core part of AgentOps is strict governance, treating agents as managed identities with scoped roles and permissions, and enforcing rigorous budget controls through per-agent quotas, alerts, and cost-aware model routing.

Third, the landscape is shifting towards a **specialized and modular toolchain** over monolithic platforms. Different tools are emerging to solve specific parts of the agentic workflow. This includes durable orchestrators like Temporal for managing long-running, resilient processes; agent-native frameworks with built-in persistence like LangGraph; CI/CD-native evaluation platforms like Braintrust that integrate directly into developer workflows; and comprehensive observability suites like LangSmith and Langfuse. The acquisition of Langfuse by ClickHouse in early 2026 underscores this trend, validating the idea that AI observability is becoming a core, distinct layer of infrastructure.

# Agent Termination Strategies

## Strategy Name

Layered Termination Framework

## Description

This is a comprehensive, consensus-based approach where the decision to terminate an agent is not based on a single signal but on a combination of multiple checks. It integrates LLM self-assessment, programmatic verification of outputs, continuous progress monitoring, and non-negotiable hard caps. The final authority to halt execution rests with the orchestrator, which evaluates these combined signals to make a robust termination decision.

## Implementation Level

Orchestrator-enforced

## Strategy Name

LLM Self-Assessment

## Description

A dedicated reasoning pass, separate from action planning, where the agent's underlying LLM is prompted to explicitly answer the question, 'Is the goal achieved?'. This step requires the model to provide evidence for its conclusion, forcing a more critical evaluation of its own progress and the state of the task.

## Implementation Level

LLM self-assessment

## Strategy Name

Programmatic Verification

## Description

An automated check to validate the agent's outputs against objective, machine-verifiable criteria. This moves beyond relying on the agent's self-report and includes checks like verifying a file's existence and content, confirming a specific state in an external API, or ensuring an output conforms to a predefined data schema. This provides a ground-truth assessment of task completion.

## Implementation Level

Orchestrator-enforced

## Strategy Name

Progress & Loop Detection

## Description

A monitoring strategy to detect when an agent is stuck or making no meaningful progress. This is often implemented by tracking the semantic similarity of the agent's recent 'thoughts' or by identifying repeated tool calls with identical arguments. If similarity is high or the state has not changed over several iterations, the orchestrator can interrupt the agent.

## Implementation Level

Orchestrator-enforced

## Strategy Name

Hard Limits & Circuit Breakers

## Description

Non-negotiable, enforced caps on execution to prevent runaway processes and control costs. These are implemented as circuit breakers that trip when a predefined limit is reached. Common limits include the maximum number of iterations or reasoning steps, total elapsed time, total monetary cost spent on API calls, and the number of consecutive errors. When a breaker trips, the agent is gracefully stopped, its state is saved, and a notification is sent.

## Implementation Level

Orchestrator-enforced

## Strategy Name

Stuck Detection with Human Escalation

## Description

A specific pattern for handling stalled agents. When progress detectors identify that an agent is stuck in a loop or not advancing, instead of immediate termination, the system checkpoints the agent's full state and routes it to a human operator for review. This allows for human-in-the-loop intervention to debug the issue or manually advance the task.

## Implementation Level

Orchestrator-enforced

## Strategy Name

Explicit State Machine

## Description

A design pattern that structures the agent's lifecycle into a formal, debuggable state machine with explicitly named states such as PLANNING, EXECUTING, WAITING_FOR_APPROVAL, PROCESSING_RESULT, and TERMINATED. Termination is treated as a final, explicit state transition, which enhances control, reproducibility, and observability of the agent's behavior.

## Implementation Level

Orchestrator-enforced


# Common Termination Failures

A primary finding from production systems is that mis-termination—either stopping too early or, more commonly, too late—is a more frequent cause of failure than the underlying LLM failing at its task. Common failure modes include:

1.  **Infinite Loops and Over-Reasoning:** Agents can become trapped in repetitive reasoning cycles without making tangible progress. This often manifests as the agent repeatedly generating semantically similar thoughts or calling the same tools with the same arguments, failing to break the loop and take a decisive action. This is a failure of progress detection mechanisms.

2.  **Runaway Execution and Cost Overruns:** This is a critical failure mode where an agent does not terminate, leading to excessive consumption of resources. Without hard limits on cost, time, or iterations, an agent stuck in a loop or pursuing an incorrect, complex path can quickly exhaust its budget, leading to significant and unexpected financial costs.

3.  **Misinterpreting Task Completion:** An agent may prematurely declare success based on a flawed self-assessment. This occurs when termination logic relies solely on the LLM's judgment without external, programmatic verification. For example, an agent might believe it has successfully saved a file or updated a record, when in reality the operation failed silently.

4.  **Failure to Recognize Infeasibility:** A common issue is an agent's inability to recognize when a task is impossible or ill-defined. Instead of terminating with an 'infeasible' status, the agent may persist indefinitely, consuming resources while trying to solve an unsolvable problem. This points to a lack of sophisticated exit logic for handling such edge cases.

# Observability Patterns Overview

The latest observability patterns for LLM and agent systems represent a significant evolution from simple logging to comprehensive, structured tracing. The core idea is to capture the entire agent execution flow as a single, coherent narrative. This is achieved by treating the entire agent run as a root span in a distributed trace. Each discrete operation within that run, such as an LLM call or a tool execution, is then recorded as a child span. This hierarchical structure allows developers to visualize the full story of the agent's decision-making process. Key metadata is captured as attributes on these spans, including `gen_ai.*` attributes for token usage (input/output), model names, and finish reasons, as well as emerging `agent.*` attributes that detail the agent's state, such as the current iteration number and the specific decision made (e.g., 'tool_call'). Furthermore, crucial decision-making logic and agent thoughts are recorded as span events, providing context for why a particular path was taken. For complex systems involving multiple collaborating agents, span links are used to connect related traces without enforcing a direct parent-child relationship, thus mapping out the intricate web of interactions.

# Opentelemetry For Agents

## Standardization Status

The OpenTelemetry (OTel) GenAI Special Interest Group (SIG) is actively leading the effort to standardize observability for AI agents. As of early 2026, a draft for the 'AI agent application semantic convention' has been established and finalized. The SIG's focus has now shifted to defining a common semantic convention for popular AI agent frameworks, including CrewAI, AutoGen, and LangGraph, to ensure that they can all report standardized metrics, traces, and logs. The OpenLLMetry project, a set of OTel-based extensions, has aligned its work with the GenAI SIG, with its own semantic conventions being incorporated into the official OpenTelemetry standard and a goal of donating its instrumentation to the OTel project.

## Key Semantic Conventions

The OpenTelemetry GenAI SIG is developing a suite of semantic conventions to provide a standardized language for describing LLM and agent operations. A key convention for agent applications has been drafted, and work is underway for agent frameworks (like CrewAI, AutoGen), LLM models, and vector databases. A practical tracing blueprint involves using specific attributes such as `gen_ai.request.model`, `gen_ai.usage.input_tokens`, `gen_ai.provider.name`, and `gen_ai.operation.name` (e.g., 'tool_call', 'agent_run'). Additionally, agent-specific attributes like `agent.decision` are used to record the agent's choices. Commercial backends like Datadog are already natively ingesting these conventions, mapping them to first-class fields in their observability platforms.

## Instrumentation Approaches

There are two primary approaches to instrumenting agents for observability. The first is through baked-in instrumentation within agent frameworks themselves (e.g., LangGraph, CrewAI), which are expected to natively emit standardized OpenTelemetry signals as the conventions are finalized. The second approach involves using external, vendor-neutral OpenTelemetry libraries and extensions, such as OpenLLMetry. This project provides OTel-based instrumentation for LLM providers and vector databases, allowing developers to instrument their applications once and send the data (in standard OTLP format) to any compatible backend like Datadog or Honeycomb. Furthermore, the OpenTelemetry Collector plays a crucial role, enabling advanced patterns like fanning out telemetry to multiple destinations (e.g., sending traces to both LangSmith and another provider) and applying processors for data redaction, enrichment, and routing before it leaves the user's network.


# Observability Platform Features

## Platform Name

LangSmith

## Key Features

LangSmith is a comprehensive observability platform with a strong focus on multi-turn evaluation capabilities and framework-agnostic tracing. It provides rich agent tracing, distributed tracing across services, and a suite of tools for insights, dashboards, and alerts.

## Opentelemetry Support

The platform offers end-to-end OpenTelemetry support, including its own SDK for instrumentation and the ability to ingest OTel data. It fully supports distributed tracing with context propagation and can use an OTel Collector for fan-out to other providers. LangSmith maps standard GenAI attributes and usage metrics, allowing it to either ingest OTel traces or export its own traces to other OTel-compatible systems.

## Cost Attribution Capabilities

LangSmith features first-class cost attribution. It automatically tracks LLM token usage and calculates costs based on a configurable model pricing map. The platform also allows users to submit custom cost data for models with non-linear or unique pricing structures. The user interface provides a detailed breakdown of costs into 'Input', 'Output', and 'Other' categories.

## Platform Name

Langfuse

## Key Features

Langfuse is an observability platform with a strong focus on agent tracing, cost analytics, and governance features like PII redaction. According to a 2026 industry analysis, its acquisition by ClickHouse has positioned it as a core piece of infrastructure for AI observability, validating the trend of observability becoming a database-centric problem.

## Opentelemetry Support

Langfuse provides OpenTelemetry integration, enabling it to perform detailed tracing of agent and LLM workflows.

## Cost Attribution Capabilities

The platform includes features specifically for cost analytics and calculators, allowing teams to monitor and analyze the financial impact of their AI applications.

## Platform Name

Braintrust

## Key Features

Braintrust is positioned as an evaluation-first and CI/CD-native platform. Its key differentiator is a dedicated GitHub Action that transforms every evaluation run into a full experiment, complete with comments on pull requests. This tight integration with developer workflows makes it popular with production teams for both evaluations and observability.

## Opentelemetry Support

The provided context emphasizes its role in tracing and experiments within a CI/CD context, implying compatibility with standard observability practices, though specific details on its OTel integration are not as elaborated as for other platforms.

## Cost Attribution Capabilities

The provided source material does not contain specific details on Braintrust's cost attribution capabilities.

## Platform Name

Arize AI / Phoenix

## Key Features

Phoenix, from Arize AI, is an observability tool specifically highlighted for being built from the ground up on open standards. It is used for agent tracing and is recognized as a key player in the landscape.

## Opentelemetry Support

Phoenix is built on OpenTelemetry and OpenInference standards, which the source material describes as having become the 'de facto standard for agent tracing.' This indicates deep, native support for and compliance with OTel conventions.

## Cost Attribution Capabilities

The provided source material does not contain specific details on the cost attribution capabilities of Arize AI's Phoenix.

## Platform Name

OpenLLMetry

## Key Features

OpenLLMetry is not a full platform but a set of vendor-neutral extensions built on OpenTelemetry. It provides instrumentation for LLM applications, including LLM providers and vector databases, designed to connect to existing observability backends like Datadog and Honeycomb.

## Opentelemetry Support

As its name suggests, OpenLLMetry is fundamentally based on OpenTelemetry. It emits standard OTLP (OpenTelemetry Protocol) data. Its semantic conventions have been influential and are being merged into the official OpenTelemetry GenAI standard, and the project aims to donate its instrumentation directly to OTel.

## Cost Attribution Capabilities

OpenLLMetry provides the foundational instrumentation to capture usage data (like token counts) which is necessary for cost calculation. However, it is designed to send this telemetry to backend platforms (like Datadog), which then perform the actual cost attribution and analysis.

## Platform Name

Proprietary Agents (Claude Code, OpenAI Codex, Devin)

## Key Features

These are closed, proprietary agent systems. Public information on their internal observability stacks is limited. They typically expose run steps and usage metrics at an API or platform level.

## Opentelemetry Support

The mainstream practice for users of these systems is to rely on the platform's provided data and integrate with third-party observability tools via SDKs or OpenTelemetry. They do not offer the same level of vendor-agnostic, platform-level OTel support as dedicated observability tools.

## Cost Attribution Capabilities

Cost is typically handled via usage metrics exposed through their APIs, which users must then process. Detailed, integrated cost attribution is more a feature of external observability platforms like LangSmith or Datadog that ingest this usage data.


# Budget Governance Approaches

## Approach Name

Monthly Quota Limit

## Description

Implements a hard financial cap on a monthly basis to prevent overall budget overruns. This acts as a top-level financial control, ensuring that total expenditure for agentic systems does not exceed the allocated budget for a given period.

## Implementation Type

Hard Cap

## Approach Name

Per-Minute Execution Throttle

## Description

Limits the number of executions or operations an agent can perform per minute. This approach is designed to prevent 'runaway' usage scenarios where an agent might enter a loop or execute an unexpectedly high volume of actions in a short time, quickly escalating costs.

## Implementation Type

API Rate Limiting

## Approach Name

Session Time Limit

## Description

Controls the total duration of an agent's session. This helps manage costs associated with long-running agents and also limits the growth of the context window, which can be a significant cost driver in itself.

## Implementation Type

Session Timeout

## Approach Name

Per-Agent/Task Budget Cap

## Description

Enforces a specific budget for individual agents or tasks. This granular control allows for precise cost management, ensuring that no single agent or task can disproportionately consume the overall budget. It often functions as a circuit breaker that halts execution upon reaching the cost limit.

## Implementation Type

Circuit Breaker / Hard Cap

## Approach Name

Token or Inference Quota

## Description

Sets a specific limit on the number of tokens or model inferences an agent can consume. This directly controls one of the primary cost drivers in LLM-based systems and provides a predictable usage boundary.

## Implementation Type

Quota System

## Approach Name

Progressive Usage Alerts

## Description

Sends notifications to stakeholders when an agent's budget consumption reaches predefined thresholds (e.g., 75%, 90%). This enables proactive monitoring and intervention before a hard cap is hit, allowing for decisions like budget reallocation or task termination.

## Implementation Type

Real-time Monitoring and Alerting

## Approach Name

Audit Trails and Central Registry

## Description

Involves logging all agent actions and maintaining a central registry of all agents, their permissions, and roles. While not a direct cost cap, this is a foundational governance approach that enables detailed cost attribution, security audits, and forensic analysis, which are essential for managing costs in an enterprise setting.

## Implementation Type

Logging / SIEM Integration


# Technical Cost Control Levers

## Lever Name

Dynamic Model Selection

## Technical Impact

This lever involves implementing a 'cost-aware planner' or routing component within the agent's architecture. This component's function is to analyze an incoming task or reasoning step and programmatically select the most appropriate LLM from a pool of available models (e.g., powerful proprietary models, cheaper open-source models, or fine-tuned private models) based on the task's complexity, required capabilities, and associated cost.

## Savings Tactic

The primary savings tactic is to optimize the cost-performance trade-off on a per-task basis. Instead of using a single, powerful, and expensive model for all operations, this approach routes simple, high-volume, or low-stakes tasks to cheaper, less powerful models. The expensive, state-of-the-art models are reserved exclusively for complex reasoning, planning, or critical tasks that require their advanced capabilities, thus significantly reducing overall token and inference costs without sacrificing performance where it matters most.


# Crash Recovery And Durability Patterns

New approaches to crash recovery and agent durability primarily revolve around two concepts: durable execution and operational circuit-breakers. Durable execution is a technique where a workflow saves its progress at key points, allowing it to be paused and resumed from the exact point it left off after a crash, timeout, or human intervention. Two prominent implementations of this are LangGraph, which uses built-in persistence and checkpointing, and Temporal, which uses a more foundational event sourcing model with a full event history acting as durable memory. Both approaches aim to make long-running agentic processes resilient to failures. Complementing this is the use of circuit-breaker patterns. These are operational guardrails implemented within the agent orchestrator to prevent runaway processes. Common triggers for these breakers include exceeding a set number of iterations, elapsed time, a predefined budget or cost, or a streak of consecutive errors. When a circuit-breaker is tripped, the agent performs a graceful stop, which involves emitting its current status, checkpointing its state for potential later analysis or resumption, and sending an alert or escalating to a human for review.

# Langgraph For Crash Recovery

## Core Concept

The main principle behind LangGraph's approach is 'Durable Execution', a technique focused on control and durability for production agents. It allows a process to save its progress at key points, enabling it to be paused and later resumed from the exact state it was in before the interruption.

## Mechanism

Durability in LangGraph is achieved through built-in persistence and checkpointing. It uses 'checkpointers' to save the state of the workflow to a durable store. This allows the system to recover the agent's state after a failure, a timeout, or a planned pause for human-in-the-loop intervention, and then resume the execution.

## Key Features

LangGraph provides several features for recovery and durability. It supports resuming runs after failures or human-in-the-loop steps. It offers different persistence modes—'exit', 'async', and 'sync'—which allow developers to trade off performance versus durability guarantees. For instance, 'sync' mode provides high durability by saving state at every step, at the cost of some performance overhead.

## Risks And Mitigations

A significant issue with LangGraph's durable execution is the 'replay risk'. When a workflow is resumed, it can re-execute steps, which may lead to duplicate side effects (e.g., sending an email twice) if the operations are not idempotent. The primary mitigation is to design workflows to be deterministic and idempotent. The documentation explicitly advises wrapping any side effects or non-deterministic operations inside 'tasks' and using idempotency keys to prevent unintended duplicate actions upon recovery.


# Temporal For Crash Recovery

## Core Concept

The core principle of Temporal's approach is 'Durable Execution' achieved through event sourcing. Every action, decision, and state change within an orchestrated process is recorded as an immutable event in a comprehensive 'Event History'. This history serves as the durable memory of the process, ensuring that its state can be fully reconstructed at any time, making it resilient to failures.

## Architecture

Temporal enforces a strict architectural separation between deterministic and non-deterministic code. The orchestration logic is encapsulated in 'Workflows', which must be deterministic. All non-deterministic operations, such as LLM calls, tool usage, or any other external API interaction, are executed as 'Activities'. The Workflow acts as the orchestrator, reliably executing these Activities while maintaining its own deterministic state.

## Recovery Mechanism

Recovery in Temporal is transparent and automatic. When a worker process crashes, a new worker can pick up the execution. It does so by replaying the recorded Event History for that workflow instance. This replay brings the workflow's state in memory back to the exact point it was at before the crash, without developers needing to write any manual checkpointing or recovery logic. The workflow then continues executing from where it left off. This mechanism also underpins features like automatic retries and timeouts for Activities.

## Benefits

This approach offers significant benefits for building robust AI agents. Developers can write 'fault-oblivious' code, as the Temporal platform handles checkpointing, retries, and recovery transparently. It has built-in, configurable retry logic for non-deterministic Activities, ensuring that transient failures are handled automatically. This makes Temporal particularly well-suited for long-running, interactive, or proactive multi-agent systems where processes may last for hours, days, or longer and must survive various types of failures.


# Missing Framework Decision Points

Common agentic frameworks currently lack several critical decision points and capabilities for production environments. In termination logic, there is a need for a standardized 'termination state taxonomy' (e.g., success, infeasible, budget_exhausted) with required evidence hooks, and common APIs for pluggable goal-completion verifiers and progress metrics. For observability, frameworks are missing standardized cross-agent span linking conventions and causality metadata, standard fields for cost attribution across different organizational units (agents, tasks, users), and portable policies for PII redaction at the collector level. In budget governance, there is a significant gap in policy-as-code solutions for hierarchical budgets (per-agent, per-user, per-org), mechanisms for dynamic budget reallocation between agents, and standardized interfaces for 'cost-aware planners'. Furthermore, few runtimes offer native budget-based circuit-breakers as a first-class termination condition. Finally, regarding crash recovery, most agent SDKs require bespoke work for idempotency primitives and compensations, lacking the baked-in features found in specialized orchestrators like Temporal. There is also a need for standardized checkpoint schemas, replay semantics, and a formal protocol for resuming execution after human-in-the-loop approvals.

# Agentic Architecture Risks

## Risk Name

Mis-termination and Silent Wrong Answers

## Description

This risk occurs when an agent incorrectly concludes it has achieved its goal, terminating prematurely with an incorrect or incomplete result. The context notes that 'mis-termination is more common than outright model failure', making it a frequent and subtle failure mode.

## Detection And Mitigation

Implement a layered termination strategy that combines multiple checks: a dedicated self-assessment step where the agent provides evidence of goal completion, programmatic verification of outputs (e.g., checking if a file exists), progress detectors, and loop detection. Establishing clear, testable success criteria upfront is critical.

## Risk Name

Over-reasoning and Infinite Loops

## Description

Agents can get stuck in a repetitive cycle of thoughts or tool calls without making tangible progress towards the goal. This 'over-reasoning' consumes significant time and resources (especially costly LLM tokens) without producing a result.

## Detection And Mitigation

Mitigation involves a combination of enforced limits and detection patterns: hard step limits, time limits, cost limits, and error streak limits. Progress can be monitored by tracking the semantic similarity of recent thoughts; if they are too similar, the agent can be interrupted. Other techniques include action-bias prompting, state hashing, and forcing an action or human review when progress stalls.

## Risk Name

Runaway Costs and Budget Overruns

## Description

Without strict controls, autonomous agents can consume an unexpectedly large amount of resources, particularly through repeated calls to expensive LLM APIs. This can lead to projects drowning 'under token costs' and failing to move beyond the prototype stage.

## Detection And Mitigation

Implement robust budget governance with hard caps and guardrails. This includes setting per-agent or per-task budget limits (in dollars), per-minute execution throttles, and session time limits to control context window growth. Employ dynamic model routing to use cheaper models for simple tasks. Set up progressive alerts at budget thresholds (e.g., 75%, 90%) to notify operators.

## Risk Name

Expanded Security Attack Surface and Identity Sprawl

## Description

As agents are granted permissions to interact with internal and external systems, they become potential targets for exploitation. A proliferation of poorly managed agents ('agent sprawl') expands the organization's security attack surface, with each agent being a potential point of failure or unauthorized access.

## Detection And Mitigation

Treat agents as first-class identities from day one. Register them in a central directory service, assign narrowly scoped roles based on the principle of least privilege, and enforce access policies like conditional access or MFA where applicable. A central 'AgentOps' function should manage the agent lifecycle, and all agent actions should be logged to a SIEM/XDR system for monitoring, auditing, and incident response.

## Risk Name

Unreliable Execution and State Loss

## Description

Agents executing long-running tasks can crash or be interrupted due to system failures or timeouts. Without a durability mechanism, the agent's entire progress and state can be lost, leaving tasks incomplete and systems in an inconsistent state.

## Detection And Mitigation

Use a durable execution framework like Temporal or LangGraph. These systems automatically checkpoint the agent's state at key points, allowing it to resume exactly where it left off after a failure. This involves separating deterministic orchestration logic from non-deterministic activities (like LLM calls) and leveraging the orchestrator's built-in retry, timeout, and recovery capabilities.

## Risk Name

Duplicate Side-Effects from Replay

## Description

Durable execution, while solving for crashes, introduces 'replay risk'. When an agent's workflow is resumed, it may re-execute steps that were already completed before the crash, leading to unintended side-effects like sending a duplicate email or making a duplicate payment if not designed carefully.

## Detection And Mitigation

Design all side-effectful operations to be idempotent. This can be achieved by using idempotency keys for API calls or wrapping non-deterministic operations inside tasks that are designed not to be re-executed on replay. Frameworks like Temporal provide clear patterns for separating deterministic workflow code from non-deterministic 'Activities' to manage this risk.


# Production Deployment Guidance

## Guidance Area

Identity and Access Management

## Recommendation

Treat agents as first-class identities.

## Implementation Details

Register agents in your organization's directory service (e.g., Active Directory). Assign narrowly scoped roles based on the principle of least privilege. Enforce security policies like conditional access and MFA where applicable to govern agent permissions and actions.

## Guidance Area

Governance and Cost Management

## Recommendation

Implement multi-level budget governance and cost controls.

## Implementation Details

Enforce hard caps on spending per-agent and per-task. Use execution throttles (e.g., per-minute limits) and session time limits to prevent runaway usage. Employ dynamic model routing to use cheaper, simpler models for high-volume tasks and reserve powerful models for complex reasoning. Set up progressive alerts at budget thresholds.

## Guidance Area

Observability and Monitoring

## Recommendation

Standardize instrumentation on OpenTelemetry for GenAI.

## Implementation Details

Instrument all agent actions using the OpenTelemetry GenAI and emerging Agent semantic conventions. Treat the entire agent run as a root span, with LLM calls and tool executions as child spans. Record standard attributes like `gen_ai.*` (token usage, model) and `agent.*` (decision, iteration). Use an OTel Collector for centralized redaction, enrichment, and routing of telemetry data.

## Guidance Area

Durability and Reliability

## Recommendation

Use a durable orchestrator for long-running or critical agent workflows.

## Implementation Details

Adopt a framework like Temporal or LangGraph (with its durable execution features) to manage agent state. This provides built-in checkpointing, retries, and recovery from failures. Architect the system to separate deterministic orchestration logic from non-deterministic activities like LLM calls, and ensure side-effectful operations are idempotent.

## Guidance Area

Operations and Lifecycle Management

## Recommendation

Establish a central 'AgentOps' function.

## Implementation Details

Create a central registry to track all deployed agents. Implement processes for lifecycle management, including deployment, versioning, and rollback. Pipe all agent telemetry and audit logs into a central SIEM/XDR for security monitoring. Define incident response playbooks specifically for agent-related issues, such as isolation and token revocation.

## Guidance Area

Termination and Control

## Recommendation

Implement layered termination logic with circuit breakers.

## Implementation Details

Combine multiple conditions for termination: agent self-assessment, programmatic verification of outputs, progress monitoring, and hard circuit breakers for iteration count, elapsed time, cost, and consecutive errors. The agent loop should be modeled as an explicit state machine (e.g., PLANNING, EXECUTING, TERMINATED) for clear, debuggable control flow.


# When Not To Use Agentic Architectures

## Scenario

Tasks that are deterministic, well-defined, and have low latency or high-volume requirements.

## Reasoning

Agentic architectures introduce significant overhead, including higher latency from reasoning loops, non-determinism from LLM responses, and higher costs from token consumption. For problems where the logic can be explicitly coded and the execution path is predictable, using an agent is inefficient and introduces unnecessary risk and unpredictability. The goal is to avoid using expensive reasoning tokens where they are not needed.

## Alternative Approach

Use traditional, deterministic software patterns. This includes well-established tools like workflow engines (e.g., using Temporal for orchestration without LLM-based activities), finite state machines, or simple, direct function calls and scripts. These alternatives provide predictable, low-latency, and cost-effective execution for well-structured problems.

