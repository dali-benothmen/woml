//! runtime script host pool.

use super::RuntimeExecutionOptions;
use crate::ScriptHostClient;
use crate::ScriptHostClientError;
use crate::ScriptHostProcessOptions;
use std::sync::Arc;

#[derive(Debug)]
struct IdleScriptHost {
  options: ScriptHostProcessOptions,
  client: ScriptHostClient,
}

#[derive(Debug, Default)]
pub(super) struct SharedScriptHostPool {
  idle: tokio::sync::Mutex<Vec<IdleScriptHost>>,
}

const MAX_IDLE_SCRIPT_HOSTS: usize = 4;

impl RuntimeExecutionOptions {
  pub fn with_shared_script_hosts(mut self) -> Self {
    self.shared_script_hosts = Some(Arc::new(SharedScriptHostPool::default()));
    self
  }
  pub(super) async fn acquire_script_host(
    &self,
  ) -> Result<ScriptHostClient, ScriptHostClientError> {
    let Some(pool) = &self.shared_script_hosts else {
      return ScriptHostClient::spawn_with_authority(
        self.script_host.clone(),
        self.capability_authority.clone(),
      )
      .await;
    };
    loop {
      let candidate = {
        let mut idle = pool.idle.lock().await;
        idle
          .iter()
          .position(|host| host.options == self.script_host)
          .map(|index| idle.swap_remove(index).client)
      };
      let Some(mut client) = candidate else {
        return ScriptHostClient::spawn_with_authority(
          self.script_host.clone(),
          self.capability_authority.clone(),
        )
        .await;
      };
      if client.is_healthy().await {
        client.set_default_authority(self.capability_authority.clone());
        return Ok(client);
      }
      client.shutdown().await;
    }
  }
  pub(super) async fn release_script_host(&self, mut client: ScriptHostClient) {
    let Some(pool) = &self.shared_script_hosts else {
      client.shutdown().await;
      return;
    };
    if !client.is_healthy().await {
      client.shutdown().await;
      return;
    }
    client.set_default_authority(None);
    let mut idle = pool.idle.lock().await;
    if idle.len() < MAX_IDLE_SCRIPT_HOSTS {
      idle.push(IdleScriptHost {
        options: self.script_host.clone(),
        client,
      });
      return;
    }
    drop(idle);
    client.shutdown().await;
  }
  pub async fn shutdown_shared_script_hosts(&self) {
    let Some(pool) = &self.shared_script_hosts else {
      return;
    };
    let clients = {
      let mut idle = pool.idle.lock().await;
      std::mem::take(&mut *idle)
    };
    for host in clients {
      host.client.shutdown().await;
    }
  }
}
