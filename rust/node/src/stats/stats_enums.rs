use serde::Serialize;
use serde_variant::to_variant_name;

/// Primary statistics type
#[repr(u8)]
#[derive(FromPrimitive, Serialize, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(rename_all = "snake_case")]
pub enum StatType {
    TrafficTcp,
    Error,
    Message,
    Block,
    Ledger,
    Rollback,
    Bootstrap,
    Network,
    TcpServer,
    Vote,
    VoteProcessor,
    VoteProcessorTier,
    VoteProcessorOverfill,
    Election,
    HttpCallback,
    Ipc,
    Tcp,
    TcpChannels,
    TcpChannelsRejected,
    TcpListener,
    TcpListenerRejected,
    Channel,
    Socket,
    ConfirmationHeight,
    ConfirmationObserver,
    Drop,
    Aggregator,
    Requests,
    RequestAggregator,
    Filter,
    Telemetry,
    VoteGenerator,
    VoteCache,
    Hinting,
    Blockprocessor,
    BlockprocessorSource,
    BlockprocessorResult,
    BlockprocessorOverfill,
    BootstrapServer,
    BootstrapServerRequest,
    BootstrapServerOverfill,
    BootstrapServerResponse,
    Active,
    ActiveStarted,
    ActiveConfirmed,
    ActiveDropped,
    ActiveTimeout,
    Backlog,
    Unchecked,
    ElectionScheduler,
    OptimisticScheduler,
    Handshake,
    RepCrawler,
    LocalBlockBroadcaster,
    RepTiers,
    SynCookies,
    PeerHistory,
    MessageProcessor,
    MessageProcessorOverfill,
    MessageProcessorType,

    BootstrapAscending,
    BootstrapAscendingAccounts,
}

impl StatType {
    pub fn as_str(&self) -> &'static str {
        to_variant_name(self).unwrap_or_default()
    }
}

// Optional detail type
#[repr(u16)]
#[derive(FromPrimitive, Serialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(rename_all = "snake_case")]
pub enum DetailType {
    All = 0,

    // common
    Ok,
    Loop,
    LoopCleanup,
    Total,
    Process,
    Processed,
    Ignored,
    Update,
    Updated,
    Inserted,
    Erased,
    Request,
    Broadcast,
    Cleanup,
    Top,
    None,
    Success,
    Unknown,
    Cache,
    QueueOverflow,

    // processing queue
    Queue,
    Overfill,
    Batch,

    // error specific
    InsufficientWork,
    HttpCallback,
    UnreachableHost,
    InvalidNetwork,

    // confirmation_observer specific
    ActiveQuorum,
    ActiveConfHeight,
    InactiveConfHeight,

    // ledger, block, bootstrap
    Send,
    Receive,
    Open,
    Change,
    StateBlock,
    EpochBlock,
    Fork,
    Old,
    GapPrevious,
    GapSource,
    Rollback,
    RollbackFailed,
    Progress,
    BadSignature,
    NegativeSpend,
    Unreceivable,
    GapEpochOpenPending,
    OpenedBurnAccount,
    BalanceMismatch,
    RepresentativeMismatch,
    BlockPosition,

    // blockprocessor
    ProcessBlocking,
    ProcessBlockingTimeout,
    Force,

    // block source
    Live,
    Bootstrap,
    BootstrapLegacy,
    Unchecked,
    Local,
    Forced,

    // message specific
    NotAType,
    Invalid,
    Keepalive,
    Publish,
    ConfirmReq,
    ConfirmAck,
    NodeIdHandshake,
    TelemetryReq,
    TelemetryAck,
    AscPullReq,
    AscPullAck,

    // bootstrap, callback
    Initiate,
    InitiateLegacyAge,
    InitiateLazy,
    InitiateWalletLazy,

    // bootstrap specific
    BulkPull,
    BulkPullAccount,
    BulkPullErrorStartingRequest,
    BulkPullFailedAccount,
    BulkPullRequestFailure,
    BulkPush,
    FrontierReq,
    FrontierConfirmationFailed,
    ErrorSocketClose,

    // vote result
    Vote,
    Valid,
    Replay,
    Indeterminate,

    // vote processor
    VoteOverflow,
    VoteIgnored,

    // election specific
    VoteNew,
    VoteProcessed,
    VoteCached,
    ElectionBlockConflict,
    ElectionRestart,
    ElectionNotConfirmed,
    ElectionHintedOverflow,
    ElectionHintedConfirmed,
    ElectionHintedDrop,
    BroadcastVote,
    BroadcastVoteNormal,
    BroadcastVoteFinal,
    GenerateVote,
    GenerateVoteNormal,
    GenerateVoteFinal,
    BroadcastBlockInitial,
    BroadcastBlockRepeat,

    // election types
    Normal,
    Hinted,
    Optimistic,

    // received messages
    InvalidHeader,
    InvalidMessageType,
    InvalidKeepaliveMessage,
    InvalidPublishMessage,
    InvalidConfirmReqMessage,
    InvalidConfirmAckMessage,
    InvalidNodeIdHandshakeMessage,
    InvalidTelemetryReqMessage,
    InvalidTelemetryAckMessage,
    InvalidBulkPullMessage,
    InvalidBulkPullAccountMessage,
    InvalidFrontierReqMessage,
    InvalidAscPullReqMessage,
    InvalidAscPullAckMessage,
    MessageSizeTooBig,
    OutdatedVersion,

    // network
    LoopKeepalive,
    LoopReachout,
    LoopReachoutCached,
    MergePeer,
    ReachoutLive,
    ReachoutCached,

    // tcp
    TcpWriteDrop,
    TcpWriteNoSocketDrop,
    TcpSilentConnectionDrop,
    TcpIoTimeoutDrop,
    TcpConnectError,
    TcpReadError,
    TcpWriteError,

    // tcp_listener
    AcceptSuccess,
    AcceptFailure,
    AcceptRejected,
    CloseError,
    MaxPerIp,
    MaxPerSubnetwork,
    MaxAttempts,
    MaxAttemptsPerIp,
    Excluded,
    EraseDead,
    ConnectInitiate,
    ConnectFailure,
    ConnectError,
    ConnectRejected,
    ConnectSuccess,
    AttemptTimeout,
    NotAPeer,

    // tcp_channels
    ChannelAccepted,
    ChannelRejected,
    ChannelDuplicate,

    // tcp_server
    Handshake,
    HandshakeAbort,
    HandshakeError,
    HandshakeNetworkError,
    HandshakeInitiate,
    HandshakeResponse,
    HandshakeResponseInvalid,

    // ipc
    Invocations,

    // confirmation height
    BlocksConfirmed,

    // request aggregator
    AggregatorAccepted,
    AggregatorDropped,

    // requests
    RequestsCachedHashes,
    RequestsGeneratedHashes,
    RequestsCachedVotes,
    RequestsGeneratedVotes,
    RequestsCannotVote,
    RequestsUnknown,

    // request_aggregator
    RequestHashes,
    OverfillHashes,

    // duplicate
    DuplicatePublishMessage,

    // telemetry
    InvalidSignature,
    NodeIdMismatch,
    GenesisMismatch,
    RequestWithinProtectionCacheZone,
    NoResponseReceived,
    UnsolicitedTelemetryAck,
    FailedSendTelemetryReq,
    EmptyPayload,
    CleanupOutdated,

    // vote generator
    GeneratorBroadcasts,
    GeneratorReplies,
    GeneratorRepliesDiscarded,
    GeneratorSpacing,

    // hinting
    MissingBlock,
    DependentUnconfirmed,
    AlreadyConfirmed,
    Activate,
    ActivateImmediate,
    DependentActivated,

    // bootstrap server
    Response,
    WriteError,
    Blocks,
    ChannelFull,
    Frontiers,
    AccountInfo,

    // backlog
    Activated,

    // active
    Insert,
    InsertFailed,

    // unchecked
    Put,
    Satisfied,
    Trigger,

    // election scheduler
    InsertManual,
    InsertPriority,
    InsertPrioritySuccess,
    EraseOldest,

    // handshake
    InvalidNodeId,
    MissingCookie,
    InvalidGenesis,

    // bootstrap ascending
    MissingTag,
    Reply,
    Throttled,
    Track,
    Timeout,
    NothingNew,

    // bootstrap ascending accounts
    Prioritize,
    PrioritizeFailed,
    Block,
    Unblock,
    UnblockFailed,

    NextPriority,
    NextDatabase,
    NextNone,

    BlockingInsert,
    BlockingEraseOverflow,
    PriorityInsert,
    PriorityEraseThreshold,
    PriorityEraseBlock,
    PriorityEraseOverflow,
    Deprioritize,
    DeprioritizeFailed,
    //
    // rep_crawler
    ChannelDead,
    QueryTargetFailed,
    QueryChannelBusy,
    QuerySent,
    QueryDuplicate,
    RepTimeout,
    QueryTimeout,
    QueryCompletion,
    CrawlAggressive,
    CrawlNormal,

    // block broadcaster
    BroadcastNormal,
    BroadcastAggressive,
    EraseOld,
    EraseConfirmed,

    // rep tiers
    Tier1,
    Tier2,
    Tier3,
}

impl DetailType {
    pub fn as_str(&self) -> &'static str {
        to_variant_name(self).unwrap_or_default()
    }
}

/// Direction of the stat. If the direction is irrelevant, use In
#[derive(FromPrimitive, PartialEq, PartialOrd, Eq, Ord, Clone, Copy, Debug)]
#[repr(u8)]
pub enum Direction {
    In,
    Out,
}

impl Direction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Direction::In => "in",
            Direction::Out => "out",
        }
    }
}

#[repr(u8)]
#[derive(FromPrimitive, Serialize, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Sample {
    ActiveElectionDuration,
    BootstrapTagDuration,
}

impl Sample {
    pub fn as_str(&self) -> &'static str {
        to_variant_name(self).unwrap_or_default()
    }
}
