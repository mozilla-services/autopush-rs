pub(crate) mod middleware;

use autoconnect_registry::ClientRegistry;

// The main AutoConnect server
pub struct Server {
    /// List of known Clients, mapped by UAID, for this node.
    pub clients: Arc<ClientRegistry>,
    /// Handle to the Broadcast change monitor
    //broadcaster: RefCell<BroadcastChangeTracker>,
    /// Handle to the current Database Client
    pub db: DbClientImpl,
    /// Count of open cells
    open_connections: Cell<u32>,
    /// OBSOLETE
    tls_acceptor: Option<SslAcceptor>,
    /// Configuration options
    pub opts: Arc<ServerOptions>,
    /// tokio reactor core handle
    pub handle: Handle,
    /// analytics reporting
    pub metrics: Arc<StatsdClient>,
}
