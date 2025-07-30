# Trigger Management

Functions for managing workflow triggers.

## `cronflow.executeManualTrigger(workflowId, payload)`

Executes a manual trigger for a workflow.

## `cronflow.executeWebhookTrigger(request)`

Executes a webhook trigger with the given request.

## `cronflow.executeScheduleTrigger(triggerId)`

Executes a schedule trigger by its ID.

## `cronflow.getTriggerStats()`

Returns statistics about trigger usage.

## `cronflow.getWorkflowTriggers(workflowId)`

Returns all triggers for a specific workflow.

## `cronflow.unregisterWorkflowTriggers(workflowId)`

Unregisters all triggers for a specific workflow.

## `cronflow.getScheduleTriggers()`

Returns all registered schedule triggers.
