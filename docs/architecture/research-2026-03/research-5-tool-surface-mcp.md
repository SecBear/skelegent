# Executive Summary

Between 2025 and 2026, the agentic AI landscape converged on a dual-layer strategy for tool integration and orchestration. The first layer is the Model Context Protocol (MCP), which solidified its position as the open, model-agnostic standard for connecting agents to tools and data, culminating in the enterprise-ready November 2025 specification and its governance under the Linux Foundation's Agentic AI Foundation. The second layer consists of powerful, platform-native agent APIs, such as OpenAI's Responses API and Anthropic's advanced tool use suite, which provide sophisticated environments for multi-tool workflows, state management, and planning. Key shifts during this period were driven by the need to manage extremely large tool surfaces. This led to new architectural patterns like dynamic, search-first discovery via the MCP Registry, lazy loading of tool definitions, and programmatic tool calling, where models generate code to orchestrate tools, dramatically reducing context bloat and inference loops. Major platforms embraced these patterns, with Anthropic pioneering the 'Tool Search Tool' and 'Programmatic Tool Calling', and OpenAI introducing its 'Responses' API with features for remote MCP server integration, client-side context compaction, and asynchronous background tasks. While significant progress was made in standardizing tool definition and discovery, key challenges remain, particularly in establishing standard metadata for tool execution policies like concurrency limits, cost estimation, and timeouts, as well as achieving unified identity and provenance across different protocols and registries.

# Key Developments Overview

## Development

MCP November 2025 Specification and Governance

## Description

The Model Context Protocol (MCP) released a major specification update on November 25, 2025, introducing enterprise-grade features like the Tasks API for long-running asynchronous operations, an improved OAuth 2.1 authorization framework, and Client ID Metadata Documents (CIMD) for identity. In December 2025, MCP was donated to the Agentic AI Foundation, a fund under the Linux Foundation, solidifying its governance and open-standard status.

## Category

Protocol Standardization

## Impact

This established MCP as a stable, governed, and feature-rich protocol for enterprise use, moving it beyond synchronous tool-calling to support complex, long-running workflows. The move to the Linux Foundation fostered widespread trust and adoption, with over 10,000 public servers emerging.

## Development

Launch of the MCP Registry

## Description

Launched in preview in September 2025, the MCP Registry is an open, official catalog and API for discovering publicly available MCP servers. It functions as a system of record, allowing clients to dynamically find tools and enabling anyone to build a compatible sub-registry.

## Category

Discovery & Ecosystem

## Impact

The registry solved the critical challenge of tool discovery at scale. It enabled a shift from static, monolithic tool manifests to a dynamic, 'search-first' architecture, where agents can discover tools on-demand, making large tool surfaces manageable.

## Development

OpenAI Responses API

## Description

Released in March 2025, the Responses API is an 'agentic by default' API from OpenAI. It natively supports multi-tool use within a single request, including built-in tools (web search, file search), custom functions, and remote MCP servers via 'connectors'. Key features include parallel tool-calling controls, a Conversations API for state management, an endpoint for client-side context compaction, and support for async background tasks.

## Category

Platform API

## Impact

Provided developers with a powerful, integrated platform for building stateful, multi-tool agents. It popularized key concepts like native support for remote protocols (MCP), explicit state management, and context compaction to enable long-running agentic interactions.

## Development

Anthropic's Advanced Tool Use (Tool Search & Programmatic Calling)

## Description

Anthropic introduced beta features to manage massive tool libraries. The 'Tool Search Tool' allows Claude to use a search function to find relevant tools on-demand, avoiding the need to load all definitions into the context. 'Programmatic Tool Calling' enables Claude to write and execute code that orchestrates multiple tools, further reducing context usage and model round-trips.

## Category

Architectural Pattern & Platform API

## Impact

Pioneered highly effective techniques for making extremely large tool surfaces practical, demonstrating context token reductions of 85-98%. This 'search-then-execute' pattern became a leading architectural approach, with other community members replicating it and achieving similar results.

## Development

Broad Adoption of MCP by Major Platforms

## Description

Throughout 2025, major AI platforms integrated support for MCP. Anthropic's Claude API could directly ingest MCP tool definitions. OpenAI's Responses API added 'connectors' for MCP servers. Google's Gemini CLI/GKE could function as an MCP server, and its A2A protocol was positioned as a complement to MCP. Microsoft's Azure OpenAI v1 also added support for remote MCP servers.

## Category

Ecosystem Integration

## Impact

This widespread adoption validated MCP as the de facto interoperability standard for agentic tools. It created a virtuous cycle where tool providers could build a single MCP server and have it be accessible across the entire AI ecosystem, accelerating innovation and tool availability.

## Development

Emergence of MCP Gateways as a Best Practice

## Description

An operational pattern emerged where a dedicated 'MCP gateway' service is used as a control plane. This gateway sits between an agent and numerous backend MCP servers, handling tasks like routing requests, parallelizing calls, aggregating results, enforcing policies (e.g., auth, rate limits), and caching.

## Category

Architectural Pattern

## Impact

Provided a scalable and manageable solution for production systems that rely on a large, federated set of tools. It separates the concerns of tool orchestration and policy enforcement from the agent's core logic, improving robustness and simplifying development.


# Mcp Evolution Timeline

## Update Date

March 26, 2025

## Version Or Milestone

Cloud-Ready Release

## Key Features Introduced

Introduction of a streamable HTTP transport and a comprehensive OAuth 2.1-based authorization framework.

## Significance

This update was critical in making MCP 'cloud-ready' by enabling modern, secure, and efficient communication over standard web protocols, moving beyond initial transport layers like stdio.

## Update Date

June 18, 2025

## Version Or Milestone

Structured Outputs & Governance

## Key Features Introduced

Introduced structured tool outputs for more reliable machine-to-machine chaining, elicitation features, and established a formal governance model. However, execution remained primarily synchronous and authorization was basic.

## Significance

Established the first stable model of the protocol, focusing on the reliability of tool interactions and laying the groundwork for community governance, though it had limitations in handling complex, long-running workflows.

## Update Date

September 2025

## Version Or Milestone

MCP Registry Preview Launch

## Key Features Introduced

Launch of the MCP Registry in preview, providing an open catalog and API for discovering publicly available MCP servers. The registry and its OpenAPI specification are open source, allowing for the creation of compatible sub-registries.

## Significance

Addressed the critical need for tool and server discovery, creating a central 'app store' like system for MCP clients to find and connect to context providers, significantly improving the ecosystem's usability.

## Update Date

November 25, 2025

## Version Or Milestone

Enterprise & Asynchronous Release

## Key Features Introduced

Introduced the Tasks API for long-running asynchronous operations, Client ID Metadata Documents (CIMD) for improved client identity and registration, steps toward statelessness, and official extensions.

## Significance

Transformed MCP from a synchronous tool-calling protocol into a robust architecture capable of supporting secure, long-running, and governed enterprise-grade workflows, addressing major limitations of the June 2025 spec.


# Mcp Ecosystem And Adoption

## Adoption Status

MCP has become the de-facto open, model-agnostic standard for connecting AI agents to external tools and data sources.

## Governing Body

Agentic AI Foundation (AAIF), a directed fund operating under the Linux Foundation, following the donation of the protocol by Anthropic in December 2025.

## Active Servers Count

10000.0

## Key Adopters

Major platforms and developer tools have adopted MCP, including ChatGPT, Claude, Gemini, Microsoft Copilot, Cursor, and Visual Studio Code.


# Agent To Agent Protocol A2A

## Protocol Name

Agent-to-Agent (A2A) Protocol

## Developer

Google

## Primary Purpose

To enable communication, discovery, and collaboration between autonomous AI agents, potentially from different vendors or platforms.

## Key Feature

The protocol likely uses a mechanism like 'Agent Cards' for agents to publish their capabilities and discover other agents, facilitating negotiation and coordination.

## Relationship With Mcp

A2A is complementary to MCP. While MCP is designed to connect agents to tools and data resources, A2A is designed to connect agents to each other for collaborative tasks.


# Openai Tool Use Evolution

## Api Or Feature

Responses API

## Description

A new API released in March 2025 designed as an 'agentic loop'. It allows the model to call multiple tools, including built-in functions, custom functions, and remote MCP servers, within a single API request. This represents a fundamental shift towards more autonomous, stateful agent behavior compared to the previous Chat Completions API.

## Key Capabilities

Agentic loop for multi-tool calls, built-in tools (web_search, file_search, computer_use, image_generation, code_interpreter), support for remote MCP servers, strict JSON Schema validation for tools, parallel tool-calling controls.

## Release Period

March 2025

## Api Or Feature

Connectors

## Description

An extension to the Responses API that provides OpenAI-maintained wrappers for popular services using the Model Context Protocol (MCP). This feature simplifies integration with third-party tools by offering pre-built, managed connections, abstracting away the complexity of individual API integrations.

## Key Capabilities

Simplified integration with popular third-party services, built on the open MCP standard, maintained by OpenAI.

## Release Period

August 2025

## Api Or Feature

Stateful Context Management (Conversations API & Compaction)

## Description

A set of features for managing context in long-running agentic conversations. The 'Conversations API' with the `store: true` parameter preserves reasoning and tool context between turns. For very long conversations, a dedicated `/responses/compact` endpoint allows developers to shrink the context sent with each turn, optimizing performance and cost.

## Key Capabilities

Stateful context (`store: true`), long-lived agent state via Conversations API, client-side context reduction via `/responses/compact` endpoint.

## Release Period

December 2025

## Api Or Feature

Asynchronous Background Tasks

## Description

Support for asynchronous, long-running tool operations, enabling more complex workflows that extend beyond a synchronous request-response cycle. This allows an agent to initiate a task and receive the results later via mechanisms like webhooks. This capability is explicitly mentioned as part of the Azure OpenAI v1 API, which maintains parity with OpenAI's offerings.

## Key Capabilities

Support for asynchronous background tasks, webhooks for results, encrypted reasoning items (in Azure implementation).

## Release Period

August 2025 onwards


# Anthropic Claude Tool Use Evolution

## Feature Name

Tool Search Tool

## Problem Solved

Context bloat from large tool libraries. The context notes that tool definitions could consume over 134K tokens, making it impractical to provide a large number of tools to the model at once.

## Mechanism

Allows Claude to use a lightweight search tool (e.g., regex, BM25, or custom embeddings) to dynamically discover and load only the most relevant tool definitions on-demand. This avoids loading the entire tool library into the context window, achieving an 85-95% reduction in token usage while maintaining access to thousands of tools.

## Api Implementation Detail

Tools or entire tool servers are marked with a `defer_loading: true` parameter, which excludes their schemas from the initial context and makes them discoverable via the search tool.

## Feature Name

Programmatic Tool Calling

## Problem Solved

High latency and excessive token usage caused by multiple API round-trips for each step in a complex task.

## Mechanism

Instead of the model making sequential tool calls, it writes and executes a piece of code within a secure execution container. This code can orchestrate multiple tool calls, implement loops and conditionals, filter large outputs, and return only the final, distilled result to the model. This dramatically reduces inference loops and context pollution.

## Api Implementation Detail

The model generates code that is executed in a 'code execution container'. This approach was described as the 'key factor that fully unlocked agent performance' and can achieve up to a 98.7% reduction in token usage.

## Feature Name

Direct MCP Tool Ingestion & Connectors

## Problem Solved

Simplifying tool interoperability and enabling the use of the open, standardized Model Context Protocol (MCP) for defining and exposing tools.

## Mechanism

Claude's Messages API can directly consume tools defined using the MCP standard. Anthropic also provides an MCP connector for remote servers and curates a 'Claude connectors directory' built on MCP, fostering a broad and open ecosystem of available tools.

## Api Implementation Detail

To ingest an MCP tool definition, a minor schema adjustment is required: the `inputSchema` field in the MCP definition must be renamed to `input_schema` to match Claude's native tool format.


# Google Vertex Ai Tool Approach

## Component Name

Agent Development Kit (ADK)

## Component Type

Development Kit

## Purpose

Used for designing and orchestrating agentic workflows, as part of the Vertex/Gemini platform.

## Component Name

Agent Engine

## Component Type

Runtime Environment

## Purpose

Promotes agent orchestration and is used for deploying custom agents to production at scale.

## Component Name

Agent2Agent (A2A) protocol

## Component Type

Communication Protocol

## Purpose

A protocol that complements the Model Context Protocol (MCP) to facilitate cross-agent discovery and coordination.

## Component Name

GKE and Gemini CLI Integration

## Component Type

Infrastructure Integration

## Purpose

Allows Google Kubernetes Engine and the Gemini Command Line Interface to be used as a Model Context Protocol (MCP) server, enabling integration with any MCP client.


# Other Major Platform Approaches

## Platform

Microsoft (Azure)

## Framework Or Service

Azure OpenAI v1 API

## Key Characteristic

Provides ongoing access to the latest features in parity with OpenAI, including the Responses API features like remote Model Context Protocol (MCP) server integration and support for asynchronous background tasks.

## Platform

OpenAI

## Framework Or Service

Responses API

## Key Characteristic

An 'agentic loop' API that allows a model to call multiple tools (built-in, custom, or remote MCP servers) in one request, and includes features for state management ('Conversations API') and context reduction ('/responses/compact' endpoint).

## Platform

Anthropic

## Framework Or Service

Claude Advanced Tool Use

## Key Characteristic

Introduced the 'Tool Search Tool' for dynamic discovery and lazy loading of tools to save context, and 'Programmatic Tool Calling' which allows the model to write and execute code to call tools, reducing inference round-trips.


# Strategies For Managing Large Tool Surfaces

The primary challenge in managing large tool libraries, often exceeding 100 tools, is 'context bloat,' where the token cost of including all tool definitions in the initial prompt becomes prohibitive. For instance, one analysis noted that tool definitions could consume as much as 134,000 tokens before optimization. To mitigate this, the state-of-the-art approach has shifted from upfront, monolithic loading of tool manifests to a dynamic, on-demand paradigm centered on discovery and efficient execution. This strategic evolution is built on two complementary layers: the Model Context Protocol (MCP) as an open, model-agnostic standard for tool connectivity, and platform-native agent APIs (like OpenAI Responses and Anthropic's tool use features) that orchestrate the process. The core of the modern strategy is to replace eager loading with 'registry-first' discovery, where agents search a catalog like the MCP Registry to find relevant tools as needed. This is coupled with lazy loading, where the full schema of a tool is only fetched and provided to the model at the moment of use. Furthermore, to reduce latency and context consumption in multi-step workflows, the pattern of 'programmatic tool calling' has emerged. Instead of the LLM mediating every single tool call in a chain, it generates a block of code that orchestrates multiple tool invocations, processes the data, and returns only a final, distilled result to the model, dramatically reducing inference round-trips and context pollution.

# Dynamic Tool Management Techniques

## Technique

Tool Search

## Description

Instead of providing all tool definitions upfront, the agent is given a lightweight search tool. When a task requires a capability not in the immediate context, the model uses this search tool to query a larger catalog of available tools based on their names, descriptions, or usage examples. The search can be implemented using methods like regex, BM25, or more advanced semantic search with embeddings.

## Primary Benefit

Dramatically reduces the initial context size by discovering tools on-demand rather than pre-loading them. This can lead to context savings of 85-95%.

## Example Implementation

Anthropic's 'Tool Search Tool,' which allows Claude to search through thousands of tools dynamically. A Google community blog post also demonstrated this pattern, achieving a 94.54% reduction in context.

## Technique

Lazy Loading / Deferred Loading

## Description

This technique involves marking specific tools or entire tool servers to be excluded from the initial context provided to the model. The full tool definitions (schemas) are only fetched and loaded into the context when the model explicitly requests them, typically after discovering them via a tool search.

## Primary Benefit

Reduces initial context size and allows for vast tool libraries to be available without overwhelming the model's context window from the start.

## Example Implementation

Anthropic's best practices recommend marking tools or MCP servers with a `defer_loading` flag to enable this on-demand loading behavior.

## Technique

Programmatic Tool Calling

## Description

The model generates and executes code within a secure sandbox to orchestrate tool usage. This code can call multiple tools, create loops, apply conditional logic, and process large outputs, returning only a final, distilled result to the model. This avoids the need for multiple, sequential inference round-trips for each step in a complex task.

## Primary Benefit

Lowers latency and significantly reduces token consumption by minimizing LLM round-trips and preventing intermediate tool outputs from polluting the context window.

## Example Implementation

Anthropic's 'Programmatic Tool Calling' feature. Another example is presenting tools as a file system, allowing the model to read definitions on-demand, which reduced token usage from 150,000 to 2,000 in one case.

## Technique

Context Compaction

## Description

A process for shrinking the conversation history for long-running agentic workflows. This involves summarizing or selectively pruning the history of tool calls and reasoning steps to maintain state without exceeding the context window limit over many turns.

## Primary Benefit

Enables long-running, stateful agent conversations by managing the growth of the context window over time.

## Example Implementation

OpenAI's `/responses/compact` endpoint, which provides a client-side mechanism to shrink the context sent with each turn in the Responses API.

## Technique

Registry-First Discovery

## Description

Utilizing a central, open catalog and API as the system of record for discovering public or private tools. Agents query this registry to find tools, rather than relying on a static, pre-configured list. This enables a scalable and discoverable ecosystem.

## Primary Benefit

Decouples agents from tool implementations and enables a dynamic, searchable ecosystem of tools that can be discovered at runtime.

## Example Implementation

The official Model Context Protocol (MCP) Registry, which launched in preview in September 2025 as a catalog for publicly available MCP servers.

## Technique

MCP Gateways

## Description

An architectural pattern where a central service acts as a control plane in front of multiple backend MCP servers. This gateway handles routing requests to the correct server, managing parallel calls, aggregating results, and enforcing cross-cutting policies like authentication, caching, and rate limiting.

## Primary Benefit

Centralizes operational concerns, simplifies policy enforcement, and improves performance by managing parallelism for systems using many distinct tool servers.

## Example Implementation

The emergence of MCP gateways as an operational best practice to federate many servers and act as a central control plane for context providers.


# Tool Composition And Validation Patterns

New patterns for tool composition and validation focus on creating more robust, efficient, and complex agentic workflows. For composition, a primary pattern is 'code-first orchestration,' where models generate and execute code within a secure sandbox. This allows for the composition of multi-tool plans that include loops, conditionals, and retries, significantly reducing the number of inference round-trips and preventing context pollution from intermediate results. This is exemplified by Anthropic's Programmatic Tool Calling. Another key pattern is the use of asynchronous task APIs, such as the MCP Tasks API (introduced in the Nov 2025 spec) and OpenAI Responses' background tasks. These enable long-running tool chains that operate beyond the limits of a single synchronous inference loop, suitable for complex, time-intensive operations. For validation, the emphasis is on ensuring reliability and safety. This starts with the enforcement of strict schemas for tool inputs and outputs, using standards like JSON Schema (as used by OpenAI) and MCP's structured tool outputs, which enable robust machine-to-machine chaining. Beyond schema validity, a new pattern is to include concrete tool use examples within the tool's definition to teach the model correct usage patterns, a practice recommended by Anthropic. Finally, systems are implementing deterministic guardrails outside the LLM loop to enforce safety, security, and policy constraints that cannot be reliably delegated to the model itself.

# Tool Metadata And Hints

The use of standardized metadata within tool schemas to provide explicit hints to an LLM agent about execution constraints is an emerging but still nascent practice. The provided research indicates that a standardized, cross-vendor specification for metadata covering concurrency limits, rate-limit budgets, execution cost, expected latency, or timeout and cancellation semantics is currently a 'gap' in the ecosystem. While major platforms and protocols have focused on discovery (identity, auth) and execution patterns (lazy loading, async tasks), they have not yet defined a shared schema for these per-tool execution policy hints. For example, while OpenAI's Responses API and the MCP Tasks API support asynchronous operations, they do not standardize how a tool can declare its specific concurrency limit or estimated cost per invocation. Currently, such policies are typically enforced through bespoke, out-of-band mechanisms, often implemented in an intermediary layer like an MCP gateway or a custom policy engine, rather than being declared directly in the tool's discoverable schema. This remains an area for future development, with potential for inclusion in future MCP specification extensions or as metadata within the MCP Registry.

# Tool Registries And Discovery Services

## Registry Name

Model Context Protocol (MCP) Registry

## Purpose

To serve as an open, central catalog and API for publicly available MCP servers, improving discoverability and implementation, functioning like an 'app store for MCP servers'.

## Sponsoring Organization

MCP open source project / Agentic AI Foundation (Linux Foundation)

## Status

Live in preview (since September 2025), progressing toward general availability.

## Registry Name

Platform-Specific Connector Directories

## Purpose

To provide curated, platform-native integrations and wrappers for popular services, often backed by MCP, ensuring compatibility and ease of use within a specific ecosystem (e.g., OpenAI or Claude).

## Sponsoring Organization

Platform providers (e.g., OpenAI, Anthropic)

## Status

Generally available, as evidenced by the mention of 'OpenAI connectors' and the 'Claude connectors directory built on MCP'.

## Registry Name

Docker and Ecosystem Catalogs

## Purpose

To supplement the official MCP Registry by providing alternative or specialized channels for discovering MCP servers and related tools.

## Sponsoring Organization

Ecosystem partners (e.g., Docker)

## Status

Active, serving as a supplementary discovery mechanism.


# Emerging Architectural Patterns

By 2026, several high-level architectural patterns have become prevalent in the design of agentic AI systems, moving definitively beyond simple linear chains of tool calls. The most significant patterns include:

1.  **Discovery-First and Lazy Loading:** This pattern fundamentally changes how agents interact with large tool surfaces. Instead of loading a monolithic manifest of all available tools into the context window, the agent first queries a discovery service, such as the MCP Registry or a custom search index. Using search techniques (e.g., BM25, embeddings) over tool names and descriptions, the agent identifies a small set of relevant tool candidates. Only the schemas for these candidates are then fetched and loaded into the context on-demand. This 'deferred loading' or 'on-demand expansion' approach, championed by Anthropic's Tool Search Tool, dramatically reduces initial context size and makes catalogs of thousands of tools practical.

2.  **Code-First Orchestration:** Rather than having the LLM mediate every single tool call in a conversational loop, this pattern has the model generate a piece of code (e.g., Python) that orchestrates a multi-step task. This code is then executed in a secure sandbox. This allows for complex logic like loops, conditionals, data filtering, and parallel execution of multiple tool calls without requiring multiple round-trips to the model. As demonstrated by Anthropic's Programmatic Tool Calling, this pattern significantly reduces token consumption, minimizes latency, and prevents the context window from being polluted with intermediate tool outputs.

3.  **Gateway as a Control Plane:** In production environments with many federated tools, an 'MCP Gateway' has emerged as a critical architectural component. This centralized service acts as an intermediary between the agent and the various tool servers. It is responsible for routing requests, handling authentication and authorization, enforcing policies like rate limiting, parallelizing calls to different servers, and aggregating the results. This pattern separates the concerns of tool management and governance from the agent's core reasoning task, leading to more robust and scalable systems.

4.  **Stateful Sessions with Active Compaction:** As agents engage in longer interactions, managing memory and state becomes crucial. Platforms like OpenAI's Responses API have established memory as a first-class primitive through its 'Conversations API', which preserves reasoning and tool context across turns. To prevent unbounded context growth, this is paired with 'active compaction'. This involves using a dedicated API endpoint (like OpenAI's `/responses/compact`) or specific model guidance to periodically summarize and shrink the conversation history, retaining salient information while discarding verbose intermediate steps. This ensures agents can maintain long-term memory without exceeding context limits.

# Security And Governance Considerations

## Guideline Or Standard

The November 25, 2025 MCP specification, which incorporates a comprehensive OAuth 2.1-based authorization framework and Client ID Metadata Documents (CIMD) for identity, under the governance of the Agentic AI Foundation (Linux Foundation).

## Design Principle

Code-first orchestration, where models generate and run code in a secure execution sandbox to invoke multiple tools, enabling complex logic while reducing inference round-trips and preventing context pollution.

## Infrastructure Solution

MCP gateways, which act as a central control plane to federate multiple MCP servers, handle routing, enforce policies, manage authentication, cache results, and handle parallel calls at scale.

## Key Risk Addressed

Unauthorized tool access and data leakage (mitigated by OAuth 2.1 and gateways), unreliable or unpredictable agent behavior (mitigated by strict JSON schemas and structured outputs), and excessive cost/latency from context pollution (mitigated by programmatic tool calling and compaction).


# Unresolved Challenges And Future Directions

## Challenge Area

Standardized Execution Policy Metadata

## Description

A significant missing component in tool surface management is a standardized, machine-readable format for communicating execution policies. This includes metadata for concurrency limits, rate-limit budgets, expected latency, timeout/cancellation semantics, cost estimates per invocation, and retry/backoff guidance. Without a standard, these critical operational parameters must be handled through bespoke implementations or enforced externally in gateway layers, rather than being discoverable as part of the tool's shared schema.

## Current Status

No standard exists. This is identified as a key gap and an area for potential future extensions to the MCP specification or as additional metadata within the MCP Registry.

