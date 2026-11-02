//! Whether a plan's host ports are free on this host.

use std::net::{Ipv4Addr, TcpListener};

use olai_stack_topology::Plan;
use serde::Serialize;

/// A host port a plan binds that something on this host already holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BusyPort {
    /// The port.
    pub port: u16,
    /// What binds it in the plan (see [`Plan::host_ports`]).
    pub owner: String,
}

/// Every host port `plan` binds that can't be bound right now, so a start can be refused
/// before Compose fails half-way. A check, not a reservation: the port can be taken between
/// the probe and the start.
pub fn probe_host_ports(plan: &Plan) -> Vec<BusyPort> {
    plan.host_ports()
        .into_iter()
        .filter(|(port, _)| !is_free(*port))
        .map(|(port, owner)| BusyPort { port, owner })
        .collect()
}

/// Whether `port` can be bound on every interface, as Docker publishes by default.
pub fn is_free(port: u16) -> bool {
    TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use olai_stack_topology::{PlanCtx, Selection, baseline_catalog};

    #[test]
    fn a_held_port_is_busy() {
        let held = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
        let port = held.local_addr().unwrap().port();
        assert!(!is_free(port));
        drop(held);
        assert!(is_free(port));
    }

    #[test]
    fn the_probe_reports_the_plans_busy_ports() {
        // Put the gateway on a port this test holds.
        let held = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
        let port = held.local_addr().unwrap().port();
        let ctx = PlanCtx {
            gateway_host_port: port,
            ..PlanCtx::default()
        };
        let plan = baseline_catalog()
            .plan(&Selection::modules(["envoy", "rustfs"]), &ctx)
            .unwrap();
        let busy = probe_host_ports(&plan);
        assert!(busy.iter().any(|b| b.port == port), "{busy:?}");
    }
}
