# WOML Switch, Custom Steps, and Providers Implementation Plan

Status: SCP0, SCP1, SCP2, SCP3, SCP5, and the executable provider-delivery
scope of SCP6 completed on 2026-08-14; source contracts,
versioned interfaces, reusable document recognition, dependency resolution,
diagnostics, folder classification, editor metadata, and durable exact-string
switch execution are implemented. Custom steps now compile into deterministic
Model v14/Definition Package v9 operations. Custom notification delivery now
runs end to end. Custom-step execution and definition-owned reusable lifecycle
hooks remain gated by the intentionally skipped SCP4 authority.

## 1. Product Outcome

This milestone adds three related capabilities without turning WOML into a
general macro language:

1. an official `<switch>` control-flow tag for readable value-based routing;
2. reusable custom steps authored as `.woml` definition files; and
3. reusable custom notification providers authored as `.woml` definition
   files.

The authoring experience should feel like importing and using a focused React
component. A reusable definition declares its props once. A workflow imports
the file under a local name and uses that name as an ordinary WOML tag.

After SCP0-SCP8 are complete, an author can:

- replace long chains of equality-based `<choose>` cases with `<switch>`;
- write one reusable step and invoke it from many workflows;
- give every invocation its own `id`, policies, props, result, and durable
  attempt history;
- build a Telegram or private-company notification provider without changing
  WOML itself;
- use a custom provider in approval and informational lifecycle notifications;
- use `on-success`, `on-error`, and `on-complete` inside reusable definitions;
- resolve props from literals, context references, and explicitly declared
  secret references;
- recover a run against the exact imported definitions it originally compiled;
  and
- receive errors that point both to the custom tag usage and its definition.

The milestone does not allow a user-defined tag to generate arbitrary workflow
structure. Official `<switch>` covers the immediate control-flow need. General
structural custom tags, templates, children, and slots remain a separately
reviewed future capability.

The normal command remains:

```bash
woml run examples/reusableDefinitionsWorkflow.woml
```

There is no separate component runner, provider runner, or second executor.
The TypeScript/Bun frontend resolves and compiles the complete source graph,
Rust remains the durable execution authority, and Bun executes JavaScript in
isolated worker contexts.

## 2. Concrete Authoring Journey

### 2.1 Reusable custom step

`examples/reusable-definitions/steps/calculate-discount.woml`:

```xml
<woml>
  <imports>
    <module name="pricing" from="./pricing.ts" />
  </imports>

  <props>
    <prop name="price" required="true" />
    <prop name="percentage" required="true" />
  </props>

  <step
    name="Calculate discount"
    description="Calculates a final price from a percentage discount">
    <script>
      return services.pricing.discount(
        props.price,
        props.percentage
      );
    </script>
  </step>

  <lifecycle>
    <on-success>
      <script>
        console.log(`Discount calculated: ${lifecycle.result.finalPrice}`);
      </script>
    </on-success>

    <on-error>
      <script>
        console.error(`Discount failed: ${lifecycle.error.code}`);
      </script>
    </on-error>

    <on-complete>
      <script>
        console.log(`Discount invocation ${lifecycle.outcome}`);
      </script>
    </on-complete>
  </lifecycle>
</woml>
```

The definition has no `id`. Identity belongs to each workflow invocation.
Its JavaScript/TypeScript imports retain the current Module System rule and are
available through `services.<moduleName>`.

### 2.2 Reusable notification provider

`examples/reusable-definitions/providers/telegram.woml`:

```xml
<woml>
  <props>
    <prop name="bot-token" required="true" secret="true" />
    <prop name="chat-id" required="true" />
  </props>

  <provider kind="notification">
    <script>
      const buttons = notification.actions
        ? {
            inline_keyboard: [[
              {
                text: "Approve",
                url: notification.actions.approve.url
              },
              {
                text: "Reject",
                url: notification.actions.reject.url
              }
            ]]
          }
        : undefined;

      const response = await services.http.request({
        method: "POST",
        url: `https://api.telegram.org/bot${props.botToken}/sendMessage`,
        json: {
          chat_id: props.chatId,
          text: notification.message,
          reply_markup: buttons
        }
      }, { name: "send-telegram-notification" });

      return {
        messageId: String(response.data.result.message_id)
      };
    </script>
  </provider>

  <lifecycle>
    <on-error>
      <script>
        console.error(`Telegram delivery failed: ${lifecycle.error.code}`);
      </script>
    </on-error>

    <on-complete>
      <script>
        console.log(`Telegram delivery ${lifecycle.outcome}`);
      </script>
    </on-complete>
  </lifecycle>
</woml>
```

`kind="notification"` is required in the first profile. The word `kind` is
kept because a future reviewed milestone may add `kind="trigger"`. This
milestone accepts only `notification`; `trigger` and every unknown value receive
an explicit unsupported-kind diagnostic.

### 2.3 Workflow usage

`examples/reusableDefinitionsWorkflow.woml`:

```xml
<woml>
  <imports>
    <module
      name="calculate-discount"
      from="./reusable-definitions/steps/calculate-discount.woml"
    />
    <module
      name="telegram"
      from="./reusable-definitions/providers/telegram.woml"
    />
  </imports>

  <workflow
    id="reusable-definitions-demo"
    name="Reusable definitions demo"
    version="1.0.0">
    <triggers>
      <manual id="start" />
    </triggers>

    <steps>
      <step id="loadOrder">
        <script>
          return {
            price: 120,
            discount: 20,
            platform: "telegram"
          };
        </script>
      </step>

      <calculate-discount
        id="discount"
        price="{{context.steps.loadOrder.price}}"
        percentage="{{context.steps.loadOrder.discount}}"
        retry="3"
      />

      <switch value="{{context.steps.loadOrder.platform}}">
        <case value="telegram">
          <approval
            id="publishApproval"
            name="Publish discounted order"
            timeout="24h"
            on-timeout="reject">
            <notify>
              <telegram
                bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}"
                chat-id="finance-team"
              />
            </notify>

            <when-approved>
              <step id="publishTelegram">
                <script>
                  return {
                    published: true,
                    finalPrice: context.steps.discount.finalPrice
                  };
                </script>
              </step>
            </when-approved>

            <when-rejected>
              <step id="rejectTelegram">
                <script>
                  return { published: false };
                </script>
              </step>
            </when-rejected>
          </approval>
        </case>

        <case value="manual">
          <step id="manualReview">
            <script>
              return { queuedForManualReview: true };
            </script>
          </step>
        </case>

        <default>
          <step id="unsupportedPlatform">
            <script>
              return {
                unsupported: context.steps.loadOrder.platform
              };
            </script>
          </step>
        </default>
      </switch>

      <step id="finish">
        <script>
          return {
            orderPrice: context.steps.loadOrder.price,
            finalPrice: context.steps.discount.finalPrice
          };
        </script>
      </step>
    </steps>
  </workflow>
</woml>
```

The reusable step behaves like an ordinary step:

```js
context.steps.discount
```

The custom provider participates in the same approval as Slack or any other
configured provider. An approval or rejection from one delivery resolves the
single durable approval. Later decisions from another provider are harmless
already-resolved responses.

The release suite uses a deterministic local provider transport. A real
Telegram example is an opt-in manual integration test so releases do not depend
on an external account or network.

## 3. Product Decisions

### 3.1 Three WOML document profiles

A `.woml` document defines exactly one runnable workflow or exactly one
reusable definition:

```text
workflow document           reusable-step document       provider document
<woml>                      <woml>                       <woml>
  <imports>?                  <imports>?                   <imports>?
  <workflow>                  <props>?                     <props>?
                                <step>                       <provider>
                                <lifecycle>?                  <lifecycle>?
                              </woml>                      </woml>
</woml>
```

`<woml>` has no attributes. A workflow's `version` remains business metadata on
`<workflow>`. No language version is added to `<woml>`.

Exactly one of `<workflow>`, top-level `<step>`, or `<provider>` is allowed.
The compiler rejects mixed documents rather than guessing the author's intent.

### 3.2 `<props>` belongs only to reusable definitions

`<props>` is a sibling before the top-level `<step>` or `<provider>`. It never
appears inside either definition.

These are compile errors:

```xml
<workflow>
  <props>...</props>
</workflow>
```

```xml
<woml>
  <props>...</props>
  <workflow>...</workflow>
</woml>
```

```xml
<step>
  <props>...</props>
</step>
```

Any `.woml` document containing `<workflow>` rejects `<props>` anywhere in that
document. Workflow data continues to enter through `context.payload`, step
outputs, services, and secrets—not workflow props.

### 3.3 Import name becomes the custom tag

The existing `<module name="..." from="..." />` declaration is reused:

```xml
<module name="calculate-discount" from="./steps/calculate-discount.woml" />
```

Source extension determines the import surface:

- `.js` and `.ts` modules use a lower-camel alias and appear under
  `services.<alias>`;
- `.woml` reusable definitions use a lowercase kebab-case alias and appear as
  an authoring tag such as `<calculate-discount />`; and
- a `.woml` import never appears beneath `services`.

The same declaration syntax avoids adding a second word such as `require`.
Kebab-case keeps custom tags visually consistent with WOML markup and avoids
case-sensitive markup surprises.

An import alias cannot collide with a built-in WOML element, another import,
or a reserved future element. The resolved document profile decides where the
tag may be used.

### 3.4 Reusable definitions are not workflows

A reusable step/provider:

- has no trigger;
- creates no independent run;
- owns no workflow ID or workflow version;
- cannot be targeted by `services.workflows.call()` or `.start()`;
- cannot be published through an event; and
- cannot be run directly.

Running a reusable definition alone returns a helpful
`WOML_DEFINITION_NOT_RUNNABLE` diagnostic that shows an import example.

When `woml run <folder>` finds workflow and reusable-definition documents, it
validates the source set, activates only workflow documents, and treats reusable
documents only as dependencies. It never creates accidental runs for them.

`services.workflows.call()` and `services.workflows.start()` remain unchanged.
They communicate with independently running workflows, not reusable imports.

### 3.5 Custom steps are ordinary durable steps after compilation

A custom step invocation requires the normal `id` and accepts the same
invocation-owned metadata and policies as a built-in `<step>`:

- `id`;
- `name` and `description`;
- `retry`, `retry-delay`, `retry-backoff`, and `retry-max-delay`; and
- future ordinary step policies only after their own contracts are executable.

The definition's top-level `<step>` omits `id`. It may provide default `name`
and `description`; invocation values override those display defaults. Retry and
other execution policies are not hidden inside the reusable definition in v1.

After lowering, Rust sees one normal logical step with:

- the invocation ID;
- the definition's script artifact;
- compiled prop expressions;
- the invocation's retry policy;
- source provenance for usage and definition; and
- optional invocation-scoped lifecycle actions.

The step uses the existing durable attempt, idempotency, timeout, cancellation,
failure, and recovery authorities. Its successful JSON return is published as
`context.steps.<invocationId>`.

### 3.6 Custom providers are supervised notification deliveries

`<provider kind="notification">` defines one notification delivery operation.
It is not merely text substitution and it does not bypass Rust.

Rust still owns:

- durable delivery intent;
- stable delivery and idempotency identity;
- retries and ambiguous-failure policy;
- approval capability creation and first-decision-wins resolution;
- cancellation and recovery;
- secret-safe events and diagnostics; and
- the rule requiring at least one successful approval notification before a
  run waits invisibly.

Bun executes the provider script using the existing isolated JavaScript worker
architecture and managed capabilities such as `services.http`.

One custom-provider tag represents one logical delivery. Sending to several
destinations inside one provider invocation is outside the safe v1 contract
because a partial external success cannot be durably reconciled. Authors repeat
the provider tag for independently supervised destinations. Built-in Slack may
retain its existing multi-channel expansion as an optimized built-in adapter.

### 3.7 `<switch>` is official WOML syntax

`<switch>` is common control flow, so users should not need to implement or
import it.

It:

- evaluates one exact `context` reference;
- requires one or more unique string-valued `<case>` arms;
- requires one final `<default>` arm;
- compares exact, case-sensitive strings with no coercion;
- selects exactly one arm;
- has no JavaScript-style fallthrough and needs no `break`;
- allows multiple flow items inside every arm; and
- reuses the existing control-choice DAG, durable selection, recovery, and
  result mechanism.

String selectors cover the primary routing use cases—status, platform, region,
category, and command—without inventing an ambiguous attribute literal type
system. Boolean decisions remain clearer with `<choose>`. Number, boolean, null,
pattern, range, and expression cases are deferred.

### 3.8 General custom structural tags remain postponed

Reusable steps cannot accept arbitrary child flow, render WOML markup, create
hidden forks, or define new join/retry/recovery semantics. Custom providers
cannot become custom triggers merely by changing a string.

This boundary keeps the compiled graph complete and reviewable before a run
starts. A future structural component system may add templates, children,
slots, and deterministic compile-time JavaScript after its grammar, security,
source mapping, and graph-expansion contracts are reviewed.

## 4. Source-Language Contract

### 4.1 Document grammar

```text
document             := workflow-document
                      | reusable-step-document
                      | provider-document

workflow-document    := <woml> imports? workflow </woml>

reusable-step-document
                     := <woml>
                          imports?
                          props?
                          reusable-step
                          reusable-lifecycle?
                        </woml>

provider-document    := <woml>
                          imports?
                          props?
                          notification-provider
                          reusable-lifecycle?
                        </woml>

imports              := <imports> module+ </imports>
module               := <module name=import-name from=relative-source />

props                 := <props> prop+ </props>
prop                  := <prop
                           name=kebab-name
                           required=boolean?
                           secret=boolean?
                         />

reusable-step         := <step reusable-step-metadata> script </step>

notification-provider
                     := <provider kind="notification"> script </provider>

reusable-lifecycle    := <lifecycle>
                          on-success?
                          on-error?
                          on-complete?
                        </lifecycle>

reusable-hook         := <hook> (script | notify)+ </hook>
```

`required` and `secret` default to `false`. Only literal `true` and `false` are
valid boolean tokens. Default prop values and prop type schemas are not included
in v1.

The top-level reusable `<step>` accepts only `name` and `description`. It does
not accept `id`, retry attributes, or timeout attributes. Those belong to the
invocation in the workflow.

The reusable lifecycle vocabulary is intentionally distinct from workflow
lifecycle scope:

- `<on-success>` after the reusable operation succeeds;
- `<on-error>` after it permanently fails; and
- `<on-complete>` after either outcome.

`<on-error>` is valid only in a reusable definition lifecycle. Workflow-level
lifecycle retains its existing `<on-failure>` vocabulary. Step-observer hooks
such as `<on-step-start>`, `<on-step-success>`, `<on-step-failure>`, and
`<on-step-complete>` are rejected inside reusable definitions.

### 4.2 Prop names and JavaScript access

Prop declarations use lowercase kebab-case:

```xml
<prop name="customer-id" required="true" />
```

WOML deterministically exposes them as lower-camel JavaScript keys:

```js
props.customerId
```

The conversion contract is fixed and collision-checked. `customer-id` and any
other spelling that would produce the same JavaScript key cannot coexist.
Reserved JavaScript keys such as `__proto__`, `prototype`, and `constructor`
are rejected.

`props` is deeply read-only. An absent optional prop is omitted from the object;
WOML does not serialize `undefined` into the compiled model or event history.

### 4.3 Prop values

At an invocation, every non-reserved attribute must match one declared prop.
Unknown props and missing required props are source-located errors.

A non-secret prop accepts:

- a literal attribute string; or
- one exact `{{context.payload...}}` or `{{context.steps...}}` reference that
  may resolve to any JSON value.

A secret prop accepts exactly one `{{secrets.NAME}}` reference. Literal secrets,
mixed interpolation, context references, and secret references passed to a
non-secret prop are rejected.

The compiled model stores value expressions and symbolic secret names, never
resolved secret values. Props are resolved immediately before the invocation
and are not added to `context`.

### 4.4 Custom-step invocation grammar

After resolving imports, an imported reusable-step alias is a flow item wherever
an ordinary `<step>` is currently legal:

```text
custom-step-invocation := <custom-step-tag
                            id=step-id
                            name=text?
                            description=text?
                            retry=positive-integer?
                            retry-delay=duration?
                            retry-backoff=retry-mode?
                            retry-max-delay=duration?
                            declared-prop-attributes
                          />
```

It may appear in root `<steps>`, switch/choose arms, fork branches, approval
arms, and parallel groups. It is empty; children are rejected because general
structural custom components are not part of this milestone.

Its `id` participates in the existing workflow-wide executable namespace and
reference rules. Reusing the same definition several times is valid when every
invocation has a distinct ID.

### 4.5 Provider usage grammar

An imported notification-provider alias is valid only as a direct child of
`<notify>`:

```text
custom-provider-usage := <custom-provider-tag
                           message=template?
                           declared-prop-attributes
                         />
```

`message` is a WOML-owned reserved attribute, not a user-declared prop:

- approval notification usage derives its standard message from approval
  metadata unless an accepted approval message override is present in the
  existing contract;
- informational lifecycle notification usage requires `message`; and
- the provider receives the final value as `notification.message`.

Custom provider tags do not accept `id`, `retry`, arbitrary lifecycle hooks, or
undeclared attributes at their use site. Stable delivery identity derives from
the owning notification, definition digest, and source order.

A step definition cannot be used inside `<notify>`. A provider definition
cannot be used inside `<steps>`. The diagnostic names both the imported alias
and the definition kind.

### 4.6 Switch grammar

Control-only form:

```xml
<switch value="{{context.steps.classify.platform}}">
  <case value="tiktok">
    <step id="publishTikTok">...</step>
  </case>

  <case value="instagram">
    <step id="publishInstagram">...</step>
  </case>

  <default>
    <step id="unsupportedPlatform">...</step>
  </default>
</switch>
```

Result-producing form:

```xml
<switch
  id="platformResult"
  name="Platform result"
  value="{{context.steps.classify.platform}}">
  <case value="tiktok">
    <step id="publishTikTok">...</step>
    <result value="{{context.steps.publishTikTok}}" />
  </case>

  <case value="instagram">
    <step id="publishInstagram">...</step>
    <result value="{{context.steps.publishInstagram}}" />
  </case>

  <default>
    <step id="unsupportedPlatform">...</step>
    <result value="{{context.steps.unsupportedPlatform}}" />
  </default>
</switch>
```

```text
switch             := control-switch | result-switch

control-switch     := <switch value=context-reference>
                        case+
                        default
                      </switch>

result-switch      := <switch
                        id=step-id
                        name=text?
                        description=text?
                        value=context-reference>
                        result-case+
                        result-default
                      </switch>

case               := <case value=non-empty-string> flow-item+ </case>
default            := <default> flow-item+ </default>

result-case        := <case value=non-empty-string>
                        flow-item+
                        result
                      </case>

result-default     := <default>
                        flow-item+
                        result
                      </default>
```

Rules:

- `value` on `<switch>` is one exact context reference, not JavaScript or a
  template;
- every `<case value>` is non-empty and unique using exact Unicode string
  equality;
- `<default>` appears exactly once and last;
- every arm contains at least one flow item;
- an ID-less switch rejects `<result>` and display metadata;
- a switch with `id` requires exactly one final `<result>` in every arm;
- only the selected arm executes;
- selection is recorded before selected work begins; and
- the result-producing form publishes only `context.steps.<switchId>` after the
  selected arm and its result resolve.

## 5. Runtime Bindings

### 5.1 Reusable-step script

The top-level reusable step script receives deeply read-only:

```text
props
context
attempt
services
```

Secret values enter only through props declared with `secret="true"`. A
reusable definition does not receive an enumerable `secrets` object. This makes
its secret dependencies explicit at every import use and prevents a reusable
file from silently reaching unrelated workflow credentials.

The script otherwise follows the existing WOML script contract: ordinary async
JavaScript, managed services, native Fetch observation, JSON-compatible return,
result limits, retry semantics, timeout/cancellation, and isolated state.

### 5.2 Provider script

A notification provider script receives deeply read-only:

```text
props
notification
attempt
services
```

Its `notification` contract is versioned before implementation and minimally
contains:

```js
notification.kind             // "approval" | "informational"
notification.message          // final message string
notification.deliveryId       // stable delivery identity
notification.idempotencyKey   // stable across safe retries
notification.actions          // approval only; absent for informational use
notification.actions.approve.url
notification.actions.reject.url
```

Internal run IDs, raw context, unrelated secrets, provider credentials, and
engine-control APIs are not exposed. Authors pass required business values as
explicit props and required credentials as secret props.

Provider scripts return a bounded JSON object. The minimal successful receipt
is `{}`; a provider may return `{ "messageId": "..." }`. Arbitrary response
bodies, tokens, approval URLs, props, and customer payloads are never persisted
as provider receipts.

Custom provider message update/edit behavior after an approval resolves is not
part of v1. Decision URLs remain single-use and return an already-resolved
response after the first decision. Built-in Slack retains its existing message
update behavior.

### 5.3 Reusable lifecycle scripts

Reusable lifecycle scripts receive:

```text
props
context                  // custom steps only; absent for providers
lifecycle
services
```

The versioned `lifecycle` object exposes only bounded outcome data:

```js
lifecycle.outcome        // "succeeded" | "failed"
lifecycle.result         // present for on-success/on-complete after success
lifecycle.error          // present for on-error/on-complete after failure
```

Lifecycle hooks execute once per logical invocation, not once per retry:

```text
success: operation result committed -> on-success -> on-complete -> continuation
failure: retries exhausted         -> on-error   -> on-complete -> failure settles
```

Hook action failures become durable warnings and do not replace the reusable
operation's already decided outcome. Business-critical work belongs in an
explicit step or provider operation, not an observational lifecycle hook.

Reusable lifecycle hooks may contain existing `<script>` and `<notify>`
actions. Import and notification cycles—including a provider whose failure hook
directly or indirectly invokes itself—are compile errors.

## 6. Compilation and Packaging Contract

### 6.1 Resolve the complete source graph before activation

The frontend recursively resolves every `.woml`, `.js`, and `.ts` dependency
inside the reviewed project boundary. It rejects missing files, path escapes,
symlink escapes, duplicate canonical sources, import cycles, document-kind
cycles, and aliases that collide after canonicalization.

Every source and transitive dependency is content-hashed and included in the
immutable Definition Package. Existing runs never reopen current project files
during recovery.

### 6.2 Compile custom tags away

The source-level custom tag is not sent to Rust as unknown markup.

```text
workflow source + imported reusable definition
                |
                v
resolve definition kind and props
                |
                v
lower to standard node + artifact + provenance
                |
                v
validate one language-neutral compiled DAG
                |
                v
Rust executes the normal durable operation
```

The compiled model retains enough definition identity and source provenance to
inspect failures and reproduce exact behavior, but it does not implement a
runtime macro expander.

### 6.3 Model and protocol versions

SCP0 freezes the exact versioned artifacts before implementation:

- **Compiled Workflow Model v14** extends Model v13 with exact-string choice
  descriptors and reserves reusable operation descriptors, compiled prop
  expressions, definition provenance, and invocation-scoped lifecycle
  definitions. Existing fork/choice/settlement graph contracts remain
  compatible.
- **Definition Package v9** stores imported `.woml` definitions and their
  transitive JS/TS artifacts beside the root workflow.
- **Script Runtime Bindings v3** adds immutable `props` for reusable step and
  reusable lifecycle execution without changing ordinary step Bindings v1 or
  workflow Lifecycle Bindings v2.
- **Custom Notification Provider Protocol v1** defines asynchronous,
  multiplexed provider-script requests, cancellation, size limits, binding
  shape, receipts, and failure taxonomy.
- **Run Event v13** retains Event v12 and adds the minimum scope required for
  reusable-invocation lifecycle progress and warnings. Standard step attempts,
  choice selection, notification delivery, approval resolution, and terminal
  outcomes keep their existing event meanings.
- **Run Inspection v5** exposes bounded reusable-definition identity,
  invocation lifecycle state, and custom-provider delivery status without
  props, outputs, messages, approval URLs, or secrets.

SCP0 must explicitly confirm whether Store v14 can persist these artifacts and
events unchanged. A Store v15 migration is added only if a real durable index
or record shape is required.

Historical Model v1-v13, Event v1-v12, Definition Package v1-v8, and Store
v1-v14 bytes remain immutable.

### 6.4 Switch lowering

`<switch>` lowers to the existing mutually exclusive choice machinery:

- one deterministic selector node;
- one ordered arm identity per case plus default;
- exact string-equality selection;
- one durable `choice_selected` event before arm execution;
- one join boundary; and
- an optional merged result node for an ID-bearing switch.

A separate `switch_selected` event is not introduced. Source kind and source
location remain diagnostic/provenance metadata; durable execution uses one
choice concept.

The selected case is never reevaluated during recovery. Switch-only workflows
may continue emitting Model v13/Event v12 when their lowered graph needs no v14
feature. A workflow using reusable definitions emits the v14/v13 package and
event profile.

### 6.5 Definition-local module isolation

A reusable definition may import local JavaScript/TypeScript modules using the
existing Module System. Those aliases are scoped to that definition. A root
workflow and an imported definition may both use `name="helpers"` without
sharing module state or resolving to the wrong artifact.

The immutable artifact may be cached, but every invocation receives the
existing fresh-state isolation guarantee. No module global or props object is
shared between attempts, runs, or invocations.

## 7. Execution Semantics

### 7.1 Custom step

1. Rust makes the invocation ready using ordinary DAG dependencies.
2. It durably starts the logical step/attempt using the existing retry identity.
3. Rust resolves context expressions and symbolic secret props at the latest
   safe moment.
4. Bun runs the frozen reusable script with `props`, `context`, `attempt`, and
   `services`.
5. Rust commits success or definitive failure before lifecycle finalization.
6. The invocation's `on-success` or `on-error` actions run, followed by
   `on-complete`.
7. After lifecycle actions settle, downstream work becomes eligible on success;
   permanent failure follows the existing workflow failure path.

Completed effects never replay. An interrupted or ambiguous external effect
retains WOML's fail-closed policy. A custom step does not weaken retry
idempotency merely because its source came from another file.

### 7.2 Custom provider

1. The approval or informational lifecycle authority creates a durable generic
   delivery intent.
2. Rust binds the exact provider definition digest, usage props, notification
   envelope, and stable idempotency key.
3. The isolated Bun provider worker executes the script through managed service
   boundaries.
4. A valid bounded receipt commits delivery success; a structured failure
   follows the existing retry/ambiguity policy.
5. Provider `on-success` or `on-error` runs, followed by `on-complete`.
6. Approval waiting begins when the existing at-least-one-delivery-success rule
   is satisfied.
7. The first valid approve/reject URL resolves the shared approval in Rust.

If Slack and Telegram are both configured, they do not create two approvals.
Each receives a separate opaque capability for the same durable request, and
the first accepted decision wins.

### 7.3 Switch

1. The selector becomes ready after its preceding route dependency.
2. Rust resolves the compiled selector reference once.
3. A non-string selector fails with `WOML_SWITCH_VALUE_INVALID` before arm work.
4. Rust chooses the exact matching case or final default.
5. It records the durable choice before any arm operation starts.
6. Only the selected arm becomes eligible.
7. Control-only flow rejoins without publishing a switch output.
8. Result-producing flow publishes the selected arm's `<result>` under the
   switch ID.

Completion order, recovery, and unrelated branches cannot change the selected
case.

## 8. Context, Result, and Visibility Contract

- A custom step sees exactly the context visibility its invocation would have
  as an ordinary step at that source location.
- Props do not add hidden dependency edges; referenced step outputs must already
  be visible and guaranteed by the compiled DAG.
- Secret props never enter `context`, step results, definition hashes, durable
  events, inspection, or logs.
- A successful custom-step return becomes `context.steps.<invocationId>`.
- A custom provider has no `context.steps` output and cannot become the workflow
  result.
- An ID-less switch publishes no output.
- An ID-bearing switch publishes exactly one path-stable
  `context.steps.<switchId>` result.
- Outputs created only inside a switch arm retain existing conditional
  visibility rules; later code should use the merged switch result when it must
  work for every case.
- The workflow's main-route result and fork settlement rules remain unchanged.

## 9. Lifecycle Composition

Reusable lifecycle hooks and workflow lifecycle hooks are separate scopes.

For a successful custom step:

```text
custom operation success
  -> reusable on-success
  -> reusable on-complete
  -> downstream workflow item
  -> eventual workflow on-success
  -> workflow on-complete
```

For a permanently failed custom step:

```text
custom operation failure after retries
  -> reusable on-error
  -> reusable on-complete
  -> workflow failure decision
  -> workflow on-failure
  -> workflow on-complete
```

The reusable definition cannot declare workflow/step observer hooks:

- `on-start`;
- `on-step-start`;
- `on-step-success`;
- `on-step-failure`;
- `on-step-complete`;
- `on-failure`;
- `on-cancel`; or
- filters naming parent workflow step IDs.

Cancellation records the normal operation cancellation and does not pretend it
is `on-error`. The reusable `on-complete` hook may run only when the existing
shutdown/cancellation budget permits safe finalization; SCP0 freezes the exact
ordering and recovery fixture before code.

## 10. Error Contract

All diagnostics retain the existing code, file, line, column, message, and
optional hint shape.

The milestone adds errors for:

- mixed or empty WOML document profiles;
- `<woml>` attributes;
- `<props>` anywhere in a workflow document or nested inside a definition;
- empty, duplicate, invalid, colliding, or reserved prop names;
- invalid `required`/`secret` booleans;
- unknown, missing, misplaced, or wrongly secret-bound props;
- a top-level reusable step with `id` or execution-policy attributes;
- invalid or unsupported provider `kind`;
- a custom step used in `<notify>` or provider used in flow;
- a custom tag that has children;
- custom aliases colliding with built-in/reserved tags;
- direct and transitive `.woml` import cycles;
- source paths escaping the project boundary;
- attempting to run a reusable definition directly;
- switch missing value/cases/default, duplicate cases, invalid order, mixed
  result profiles, non-string runtime selectors, or unsupported attributes;
- provider receipts that are non-JSON, oversized, or expose forbidden fields;
- reusable lifecycle hooks or actions outside the allowed profile;
- lifecycle notification cycles;
- malformed Model v14 definition/provenance/prop/lifecycle descriptors;
- contradictory Event v13 reusable lifecycle history; and
- recovery attempting to substitute a changed current definition.

When an error originates inside an imported definition, the diagnostic chain
shows:

1. the root workflow and custom-tag usage location;
2. the import declaration and resolved path; and
3. the exact definition/module source location.

No diagnostic prints resolved props, messages, approval URLs, response bodies,
or secret values.

## 11. Production and Operational Behavior

- `woml check` validates workflow and reusable-definition files without running
  them and explains each document kind.
- `woml run <folder>` activates only workflows after validating and atomically
  pinning their complete dependency graphs.
- `woml get <runId>` exposes custom step/provider aliases and statuses without
  their props or results.
- `woml inspect` uses bounded counts for reusable invocations and provider
  delivery health; import aliases do not become unbounded metric labels.
- structured logs may include workflow ID, run ID, invocation ID, definition
  digest prefix, provider alias, delivery ID, and safe failure code.
- backup/restore includes exact `.woml` definition artifacts and transitive
  modules.
- retention never removes a definition artifact referenced by a retained run.
- pruning unused definitions follows the existing reference-safe artifact
  rules.
- background runtime readiness fails atomically if any activated workflow has
  an invalid or missing reusable dependency.
- secret scanning covers prop bindings, provider requests/responses, lifecycle
  warnings, diagnostics, logs, snapshots, and crash paths.

## 12. Implementation Phases

### 12.1 Phase summary

| Phase | What changes | Result after the phase |
| --- | --- | --- |
| SCP0 — completed | Freeze syntax, props, switch, model, event, provider protocol, lifecycle, errors, and reviewed fixtures. | Every layer targets one approved design before code changes. |
| SCP1 — completed | Add the three document profiles, `.woml` import resolution, prop declarations, and source diagnostics. | WOML can understand reusable definitions and reject invalid usage without executing them. |
| SCP2 — completed | Implement official `<switch>` validation, lowering, durable selection, results, and recovery. | Authors can run readable string-based routing through the existing Rust choice engine. |
| SCP3 — completed | Compile reusable-step imports into Model v14 and Definition Package v9 with immutable props and provenance. | A custom step becomes one deterministic engine-ready operation. |
| SCP4 | Execute custom steps with retries, services, secrets, results, lifecycle hooks, and recovery. | Imported custom steps work like native durable steps. |
| SCP5 — completed | Compile custom notification providers and freeze the provider-worker boundary. | Custom provider tags lower to generic supervised delivery definitions. |
| SCP6 — provider delivery completed | Execute custom providers for approvals and workflow lifecycle notifications with shared decisions and safe retries. Definition-owned lifecycle hooks remain fail-closed until SCP4 supplies Event v13 authority. | A real user-authored notification provider works end to end without silently ignoring unsupported hooks. |
| SCP7 | Complete composition, CLI, folder activation, operations, cancellation, backup, and compatibility. | The features work inside production automations rather than isolated demos. |
| SCP8 | Harden, benchmark, document, package, and publish the milestone. | Switch, custom steps, and custom notification providers are supported WOML features. |

Phase labels are planning shorthand only. Permanent test names, fixture names,
scripts, modules, and source symbols use descriptive product/behavior names—not
filenames such as `scp4_*`.

### SCP0 — Freeze contracts and reviewed fixtures — completed

Changes:

- Freeze the three exact top-level document grammars and child order.
- Freeze `.woml` import alias, path, document-kind, cycle, and project-boundary
  rules.
- Freeze prop declarations, kebab-to-camel mapping, allowed value expressions,
  secret bindings, reserved attributes, and runtime omission behavior.
- Freeze custom-step invocation attributes, result identity, retries,
  cancellation, and lifecycle ordering.
- Freeze `kind="notification"`, provider runtime bindings, receipt shape,
  retry/ambiguity behavior, and shared approval semantics.
- Reserve but reject `kind="trigger"` until a future reviewed milestone.
- Freeze both switch profiles, exact string matching, default behavior, result
  merging, event reuse, and recovery.
- Review Model v14, Definition Package v9, Script Bindings v3, Event v13,
  Inspection v5, and Custom Notification Provider Protocol v1 schemas.
- Decide explicitly whether Store v14 is sufficient.
- Add source, compiled-model, package, binding, event-history, inspection,
  recovery, and diagnostic fixtures.
- Include success/failure/cancellation/retry fixtures for custom step and
  provider lifecycle.
- Include switch case/default/result/recovery fixtures.
- Include one approval with built-in Slack plus a custom provider to prove one
  shared decision.
- Include historical Model v1-v13/Event v1-v12/Package v1-v8 fixtures.

Reuse:

- Model v13 DAG/fork/choice visibility and settlement.
- Event v12 folding and fail-closed attempt recovery.
- Definition Package v8 artifact hashing and source maps.
- Module System resolver, bundler, isolation, service instrumentation, and
  generated types.
- Retry/idempotency, notification, approval, lifecycle, runtime policy,
  cancellation, backup, and retention authorities.

Result:

The expensive interfaces are reviewable together before implementation can
silently choose incompatible defaults.

Gate:

SCP1 does not begin until all reviewed source files, schemas, expected compiled
graphs, durable histories, binding snapshots, error snapshots, and compatibility
fixtures agree.

### SCP1 — Parse reusable documents, imports, and props — completed

Changes:

- Extend the frontend document classifier for workflow, reusable-step, and
  notification-provider profiles.
- Reject `<props>` in every document containing `<workflow>`.
- Parse and validate `<props><prop ... /></props>` with exact ordering and
  source locations.
- Extend `<module>` resolution to local `.woml` reusable definitions while
  retaining `.js`/`.ts` service imports.
- Apply extension-specific alias grammar and collision checks.
- Resolve recursive dependency graphs and reject cycles/path escapes.
- Teach `woml check` and folder discovery to classify definitions without
  activating them.
- Generate editor completions for imported custom tags and their declared props
  during normal `woml check`/`woml run`; no manual type-generation command.
- Keep compilation/execution behind explicit SCP3/SCP5 feature gates.

Result:

WOML recognizes reusable source files, gives component-like prop completions,
and explains invalid definitions/usages without claiming they execute.

Gate:

Valid source graphs classify deterministically; invalid structure, props,
aliases, paths, and cycles fail at the responsible source token; existing
JS/TS module imports remain unchanged.

### SCP2 — Build official switch execution — completed

Changes:

- Add `<switch>`, `<case>`, and `<default>` to recursive flow validation.
- Validate control-only and result-producing profiles.
- Validate exact selector references, string case values, uniqueness, arm
  order, non-empty arms, and result cardinality.
- Lower switch arms to deterministic existing choice nodes/edges.
- Record selection through the existing durable `choice_selected` authority.
- Preserve selection through restart without reevaluation.
- Map runtime non-string values and selected-arm failures to switch source
  locations.
- Support switch on the main route, in forks, approvals, and other currently
  legal recursive flow locations.
- Add editor declarations, syntax docs, and a manually runnable example.

Result:

Authors can replace repeated equality conditions with a compact official tag,
and Rust executes only the selected case.

Gate:

Case, default, merged-result, nested composition, failure, cancellation, and
recovery tests pass through the packaged Rust engine. Reversed timing cannot
change selection or result.

### SCP3 — Compile reusable custom steps — completed

Changes:

- Extend the existing Model v14 TypeScript and Rust switch profile with
  reusable-operation validation.
- Add Definition Package v9 reusable-definition artifacts and dependency
  manifests.
- Lower each custom-step usage into one ordinary step node with compiled prop
  expressions, definition digest, script artifact, and dual source provenance.
- Add Script Runtime Bindings v3 for immutable `props`.
- Namespace definition-local JS/TS modules without exposing generated names.
- Merge invocation-owned name/description defaults and retry policies.
- Statically validate context visibility and symbolic secret dependencies.
- Produce stable definition hashes independent of absolute paths and file
  timestamps.
- Keep Rust execution gated until SCP4.

Result:

The frontend produces one deterministic, language-neutral operation for every
custom step usage, and Rust can independently validate it.

Gate:

TypeScript and Rust accept the same reviewed Model v14 fixtures, reject every
malformed prop/artifact/provenance fixture, and reproduce stable hashes across
clean directories.

### SCP4 — Execute custom steps and their lifecycle

Changes:

- Register frozen reusable artifacts with the long-lived Bun host.
- Resolve props and secrets per attempt without persisting values.
- Execute definition scripts in fresh isolated invocation state.
- Reuse services, native Fetch instrumentation, operation events, limits,
  cancellation, and retry/idempotency behavior.
- Persist successful results under the invocation ID.
- Add Event v13 reusable lifecycle scope and pure projection folding.
- Execute `on-success`/`on-error` followed by `on-complete` exactly once per
  logical invocation.
- Record lifecycle action failures as warnings without rewriting operation
  truth.
- Recover completed, pending, retry-waiting, lifecycle-finalizing, interrupted,
  and ambiguous invocations correctly.
- Map errors through both workflow usage and definition/module source maps.

Result:

An imported custom step runs like a native durable step and exposes its result
through `context.steps.<id>`.

Gate:

The acceptance custom step passes success, permanent failure, retry,
idempotency, services, secret, lifecycle, cancellation, crash, and recovery
tests with no secret or prop leakage.

### SCP5 — Compile custom notification providers — completed

Changes:

- Validate provider definitions and `kind="notification"`.
- Validate provider use only as a direct `<notify>` child.
- Lower each use to one generic provider delivery with a stable identity,
  compiled props, definition digest, and notification domain.
- Extend Model v14 notification definitions beyond the current Slack-only model
  validator without weakening built-in Slack validation.
- Freeze and implement both sides of Custom Notification Provider Protocol v1.
- Make the protocol asynchronous/multiplexed with invocation IDs, UTF-8 byte
  framing, cancellation, context/result limits, and structured failures.
- Register provider artifacts through immutable Definition Package v9.
- Validate receipt redaction and forbid arbitrary response persistence.
- Keep real execution gated until SCP6.

Result:

The complete custom-provider request is deterministic, provider-neutral,
secret-safe, and independently validated by TypeScript, Rust, and Bun.

Gate:

Protocol conformance fixtures cover multibyte text, literal CRLF content,
out-of-order responses, cancellation, oversized messages, malformed receipts,
unknown artifacts, and every failure kind.

### SCP6 — Execute custom providers end to end — provider delivery completed

Implementation note: approval delivery, workflow lifecycle delivery, durable
decisions, retries, recovery primitives, redaction, and local/Telegram examples
are implemented. A provider definition's own `on-success`, `on-error`, and
`on-complete` actions depend on the reusable lifecycle/event authority assigned
to SCP4, which was intentionally skipped. `woml run` therefore rejects that
profile with `WOML_REUSABLE_LIFECYCLE_EXECUTION_UNAVAILABLE` instead of silently
ignoring hooks. Completing SCP4 will close this remaining part of the original
SCP6 scope.

Changes:

- Run provider scripts through isolated Bun workers under Rust notification
  supervision.
- Expose `props`, `notification`, `attempt`, and managed `services` only.
- Bind secret props at invocation time and redact every boundary.
- Reuse stable notification delivery identity, safe retries, ambiguous failure,
  and recovery.
- Support approval notifications with shared approve/reject capabilities.
- Support informational workflow and reusable lifecycle notifications.
- Execute provider `on-success`/`on-error` and `on-complete` without changing
  delivery truth.
- Preserve built-in Slack behavior and message updates unchanged.
- Add a deterministic local provider acceptance fixture and an opt-in real
  Telegram example.

Result:

A user can author, import, and execute a real custom notification provider
without modifying WOML or installing a large runtime SDK.

Gate:

One approval delivered through Slack and a custom provider resolves exactly
once from either channel, recovers across restart, never waits after every
delivery fails, and never logs credentials or approval capabilities.

### SCP7 — Complete composition and production operations

Changes:

- Test custom steps inside switch/choose, parallel, fork branches, approval
  arms, retries, and Workflow Calls/Starts.
- Test custom providers inside approval and workflow/reusable lifecycle hooks.
- Detect direct and transitive lifecycle notification cycles.
- Propagate run cancellation, shutdown, and workflow timeout into provider and
  custom-step workers.
- Make multi-file and folder activation validate/pin source graphs atomically
  while activating only workflows.
- Add Inspection v5, safe logs, bounded metrics, and actionable CLI errors.
- Preserve reusable artifacts through background runtime restart,
  backup/restore, and reference-safe retention/pruning.
- Verify runtime policy admission and concurrency limits include custom work.
- Preserve workflow-call targeting: reusable definitions are never candidates.

Result:

Switches and reusable definitions work in the same production runtime and
operational lifecycle as native WOML features.

Gate:

Production integration tests cover mixed constructs, folder activation,
background restart, cancellation, timeout, inspection, backup/restore,
retention, and clean shutdown.

### SCP8 — Harden, package, document, and publish

Changes:

- Run adversarial parser/import/props/protocol/artifact/source-map/security
  suites.
- Test one/many imports, repeated invocations, deep dependency graphs, alias
  collisions, cycles, missing files, changed/deleted sources, and corrupted
  stored artifacts.
- Test switch at every legal recursive position and every invalid form.
- Test crashes before/after custom step result, lifecycle action, provider
  intent, provider response, approval wait, decision, and workflow settlement.
- Benchmark cold/warm compilation, artifact caching, switch selection, custom
  step invocation, provider startup, and many-definition folders.
- Verify historical models, events, packages, stores, workflows, modules,
  choices, notifications, and production operations unchanged.
- Update language, architecture, services, notifications, lifecycle, module,
  migration, deployment, security, editor, and CLI documentation.
- Add descriptive independent release commands and include them in the full
  repository release gate.
- Test from a clean installed package with the Rust engine and Bun hosts.

Result:

Official switch, reusable custom steps, and reusable custom notification
providers become supported and publishable WOML features.

Gate:

Frontend, Rust, N-API, Bun hosts, protocol/schema conformance, typecheck,
Clippy, clean-package, compatibility, recovery, security, secret scan, and
performance gates pass without skipped native coverage.

## 13. Verification Matrix

| Area | Required proof |
| --- | --- |
| Documents | Workflow, reusable-step, and provider documents classify exactly; mixed profiles fail clearly. |
| Props placement | Props work only at reusable document top level and fail anywhere in a workflow document or inside a definition tag. |
| Props binding | Required/optional/secret props resolve correctly; unknown, missing, colliding, and unsafe bindings fail. |
| Imports | `.woml` tags and JS/TS service modules use one declaration syntax with kind-correct aliases and no cycles/path escapes. |
| Switch | Exact string case/default selection, no fallthrough, merged results, errors, and recovery are deterministic. |
| Custom step result | A successful result appears only under the invocation ID and respects normal visibility. |
| Custom step attempts | Retry, idempotency, cancellation, timeout, ambiguity, and recovery match native steps. |
| Provider delivery | Intent precedes execution; receipts are bounded; retries and ambiguous failures retain the existing safe policy. |
| Shared approval | Built-in and custom providers resolve one approval once; later decisions are already-resolved. |
| Lifecycle | Reusable hooks run once per logical operation in success/error then complete order; warnings do not rewrite truth. |
| Services | Managed services and native Fetch preserve current tracking, limits, and cancellation inside reusable scripts. |
| Secrets | Only declared secret props resolve; no value reaches packages, models, events, context, logs, inspection, or errors. |
| Isolation | Props and module globals do not cross attempts, invocations, or runs. |
| Source maps | Failures identify the workflow usage, import, definition, and JS/TS source location. |
| Folder run | Only workflow documents activate; reusable definitions validate and pin as dependencies. |
| Operations | Inspection, logs, metrics, backup/restore, retention, prune, shutdown, and readiness understand reusable work. |
| Compatibility | Historical models/events/packages/stores and workflows without reusable imports behave unchanged. |
| Package | A clean installed CLI runs the reviewed examples through Rust and Bun. |

## 14. Explicit Non-Goals

This milestone does not add:

- arbitrary structural custom tags;
- compile-time `<render>`, templates, loops, children, or slots;
- custom tags that expand into hidden steps/forks/approvals;
- runtime mutation of the compiled workflow graph;
- `.woml` workflow embedding or workflow imports;
- child-workflow communication beyond existing
  `services.workflows.call()`/`.start()`;
- provider `kind="trigger"` execution;
- custom inbound callback/signature protocols;
- native Telegram callback buttons that bypass the existing decision URLs;
- custom-provider message editing after decision;
- multi-destination delivery inside one custom-provider invocation;
- default prop values or a prop type/schema language;
- numeric, boolean, null, range, pattern, or expression switch cases;
- JavaScript-style switch fallthrough;
- hidden retry policies inside reusable definitions;
- installed/package-registry components;
- remote component URLs;
- untrusted multi-tenant JavaScript containment claims;
- new built-in Discord, WhatsApp, or Telegram adapters; or
- retirement of the JavaScript chaining SDK during this milestone.

These items are not silently implemented through convenience behavior. Each
requires its own reviewed product and compatibility contract.

## 15. Expected File Areas

| Area | Expected locations |
| --- | --- |
| Language/parser/compiler | `woml/src/parser.ts`, `woml/src/compiler.ts`, `woml/src/model.ts` |
| Definition resolution/package | `woml/src/modules.ts` or a focused reusable-definition resolver, Definition Package v9 schemas |
| Script analysis/bindings | WOML script analysis, generated declarations, Script Bindings v3 fixtures |
| Rust model/event/projection | `core/woml-engine/src/model.rs`, `event.rs`, `projection.rs` |
| Rust scheduling/recovery | `core/woml-engine/src/engine.rs`, `runtime.rs`, `durable.rs` |
| Notification authority | existing Rust notification scheduling/delivery and approval resolution areas |
| Bun execution | existing script host/module worker plus a focused custom-provider invocation adapter |
| Native/CLI | `core/src/woml_bridge.rs`, `woml-cli/src/rust-executor.ts`, `woml-cli/src/cli.ts` |
| Operations | run inspection, terminal inspector, logs/metrics, backup/restore, retention/prune |
| Protocols/schemas | `docs/protocols`, `docs/schemas`, reviewed JSON/WOML/event fixtures |
| Documentation/editor | `docs/woml-v0.1.md`, architecture, notification/lifecycle/module guides, editor data |
| Examples | reusable step, local provider, opt-in Telegram provider, and switch workflow examples |
| Release gates | descriptive frontend, Rust, CLI, protocol, package, recovery, and benchmark tests |

Implementation must extend the existing compiler, DAG, event fold, scheduler,
script host, module artifact, notification, lifecycle, and production-runtime
authorities. It must not create a parallel custom-step executor or make Bun the
durable truth owner.

## 16. Risks and Guardrails

### Reusable syntax can accidentally become an unbounded macro language

The first profile exports exactly one script-backed step or one notification
provider. Custom tags are empty at their use site and cannot generate workflow
structure.

### A reusable step can hide important behavior

Invocation identity and policies remain visible at the use site. Definition
metadata, imports, required props, secrets, and source path are inspectable.
Lifecycle actions are observational and cannot silently rewrite business truth.

### Secret props can leak through user JavaScript

Only explicit symbolic secret references resolve. Workers receive the minimum
declared values, runtime boundaries redact them, results/receipts are scanned,
and docs warn that returning or deliberately transmitting a secret remains an
author-controlled effect.

### A provider script can partially perform an external effect

Rust records intent first, supplies a stable key, retries only definitive safe
failures, and fails closed on ambiguity. One tag is one delivery; v1 does not
hide multiple destinations behind one attempt.

### Custom provider flexibility can weaken approval security

Rust creates and validates opaque single-use decision capabilities. Provider
code can transport URLs but cannot decide, reopen, or mutate an approval.

### Props can create hidden data dependencies

Every context-valued prop remains a compiled reference checked against DAG
visibility and dominance. Props never infer or add edges.

### Import aliases can collide with source-language tags

Built-in and reserved names are closed sets. Custom tags use lowercase
kebab-case and are resolved before flow validation with source-located
collision errors.

### Recovery can load current files instead of frozen definitions

The Definition Package stores every imported `.woml` and transitive module by
digest. Existing runs use stored bytes only.

### Switch can introduce surprising coercion

The first profile accepts only string selectors and exact case-sensitive string
matching. Other types fail instead of coercing.

## 17. Global Roadmap After This Milestone

1. **Additional Communication Providers** — add built-in Discord, WhatsApp, and
   Telegram triggers, notifications, and messaging capabilities when product
   demand justifies them; review `provider kind="trigger"` as part of an inbound
   provider contract rather than enabling it by string alone.
2. **Retire the JavaScript Chaining SDK** — remove the old SDK only after WOML
   reaches sufficient parity and users have a supported migration path.

Completed milestones—including choices, parallel, fork/branch, Human Approval,
retries/idempotency, production triggers, services/capabilities, the essential
Module System, Durable Workflow Calls and Start, lifecycle/engine controls,
runtime policies, Durable State, and Production Runtime/Operations—remain the
baseline and are not repeated as future work.

## 18. Definition of Done

The milestone is complete only when:

- `<switch>` is official, documented, durable, recoverable, and usable in every
  reviewed recursive flow location;
- control-only and result-producing switch profiles have deterministic output;
- `.woml` documents classify as exactly one workflow, reusable step, or
  notification provider;
- `<woml>` has no version or other attributes;
- `<props>` is top-level only in reusable definitions and is rejected in every
  workflow document;
- imported `.woml` names become kebab-case custom tags without entering
  `services`;
- custom step invocations use normal step IDs/policies and publish normal step
  results;
- custom provider definitions require `kind="notification"` and work in
  approval and informational lifecycle notifications;
- provider `kind="trigger"` is reserved but non-executable;
- reusable definitions support only `on-success`, `on-error`, and
  `on-complete`, with no author-facing `on-step-*` hooks;
- all props, contexts, services, secrets, attempts, notification values, and
  lifecycle values follow reviewed immutable binding schemas;
- Rust remains the one durable execution/notification/approval authority;
- Definition Package v9 pins every source needed for recovery;
- Model v14/Event v13 histories validate, fold, persist, recover, inspect, back
  up, restore, retain, and prune safely;
- direct runs of reusable files fail helpfully while folder runs activate only
  workflows;
- historical models/events/packages/stores and existing workflows remain
  compatible;
- errors point through usage, import, definition, and module locations without
  leaking data; and
- the independent release gate and full repository release gate pass from a
  clean installed package.

## 19. SCP0 Review Gate

Before SCP1 begins, review these artifacts together:

- canonical workflow, reusable-step, and provider source fixtures;
- every invalid mixed-document and props-placement fixture;
- `.woml` import alias/path/kind/cycle fixtures;
- control-only and result-producing switch sources and compiled graphs;
- custom-step Model v14 nodes with literal, context, and secret props;
- custom-provider Model v14 delivery definitions;
- Definition Package v9 dependency manifests and stable hashes;
- Script Bindings v3 and provider-runtime binding snapshots;
- Custom Notification Provider Protocol v1 request/response/failure fixtures;
- Event v13 success, error, lifecycle-warning, cancellation, and recovery
  histories;
- Run Inspection v5 redacted snapshots;
- built-in Slack plus custom-provider shared approval histories;
- folder activation with workflow and reusable files;
- failure/cancellation/shutdown/recovery ordering tables;
- the Store v14/no-migration decision or an explicitly reviewed Store v15;
- source diagnostic catalog with import/definition chains; and
- historical Model v1-v13, Event v1-v12, Definition Package v1-v8, and Store
  v1-v14 compatibility proof.

SCP0 is approved only when the same artifacts answer:

1. Is this file runnable or reusable?
2. Where is an imported custom tag legal?
3. Which props and secrets does it receive?
4. What exact durable operation does it become?
5. What result, if any, becomes visible to later steps?
6. When do its lifecycle hooks run, and can they change business truth?
7. Who owns delivery, retry, approval, and recovery for a custom provider?
8. How is a switch case chosen and preserved after restart?
9. Which exact source bytes execute after the project files change?
10. What does the user see when validation or execution fails?

No implementation phase may answer one of these differently from the frozen
contracts.
