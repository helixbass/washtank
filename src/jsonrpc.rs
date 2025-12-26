// from scook12/rust-lsp

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, BufReader},
    process::ChildStdout,
    sync::{mpsc, oneshot, RwLock},
};

pub struct Client<R, W> {
    /// The underlying transport for reading and writing messages.
    transport: Arc<RwLock<Transport<R, W>>>,
    /// Counter for generating unique request IDs.
    request_id_counter: AtomicI64,
    /// Channel for receiving incoming messages.
    message_receiver: Option<mpsc::UnboundedReceiver<RpcMessage>>,
    /// Channel for sending outgoing messages.
    #[allow(dead_code)]
    message_sender: mpsc::UnboundedSender<RpcMessage>,
    /// Handle for the message processing task.
    _message_task: tokio::task::JoinHandle<()>,
}

impl<R, W> Client<R, W>
where
    R: AsyncRead + Unpin + Send + Sync + 'static,
    W: AsyncWrite + Unpin + Send + Sync + 'static,
{
    /// Create a new LSP client with the given transport.
    pub fn new(reader: R, writer: W) -> Self {
        let transport = Arc::new(RwLock::new(Transport::new(reader, writer)));
        let (message_sender, message_receiver) = mpsc::unbounded_channel::<RpcMessage>();
        let message_sender_clone = message_sender.clone();

        // Spawn task to handle incoming messages
        let transport_clone = Arc::clone(&transport);
        let message_task = tokio::spawn(async move {
            loop {
                let message = {
                    let mut transport = transport_clone.write().await;
                    match transport.read_message().await {
                        Ok(msg) => msg,
                        Err(e) => {
                            // log::error!("Failed to read message: {}", e);
                            break;
                        }
                    }
                };

                let rpc_message = match message.parse_rpc_message() {
                    Ok(msg) => msg,
                    Err(e) => {
                        // log::error!("Failed to parse RPC message: {}", e);
                        continue;
                    }
                };

                match &rpc_message {
                    RpcMessage::Response(response) => {
                        if let Some(id) = &response.id {
                            let mut pending = pending_requests_clone.write().await;
                            if let Some(pending_request) = pending.remove(id) {
                                if let Err(e) = pending_request.sender.send(response.clone()) {
                                    // log::warn!(
                                    //     "Failed to send response to pending request: {:?}",
                                    //     e
                                    // );
                                }
                            } else {
                                // log::warn!("Received response for unknown request ID: {}", id);
                            }
                        }
                    }
                    RpcMessage::Request(_) | RpcMessage::Notification(_) => {
                        // Forward to client for handling
                        if message_sender_clone.send(rpc_message).is_err() {
                            // log::error!("Message receiver dropped, stopping message loop");
                            break;
                        }
                    }
                }
            }
        });

        Self {
            transport,
            request_id_counter: AtomicI64::new(1),
            pending_requests,
            message_receiver: Some(message_receiver),
            message_sender,
            _message_task: message_task,
        }
    }

    /// Generate a new unique request ID.
    fn next_request_id(&self) -> Id {
        Id::Number(self.request_id_counter.fetch_add(1, Ordering::SeqCst))
    }

    /// Send a request and wait for the response.
    pub async fn send_request(
        &self,
        method: impl Into<String>,
        params: Option<serde_json::Value>,
    ) -> Result<ResponseMessage> {
        let id = self.next_request_id();
        let request = match params {
            Some(params) => RequestMessage::with_params(id.clone(), method, params),
            None => RequestMessage::new(id.clone(), method),
        };

        let (response_sender, response_receiver) = oneshot::channel();

        // Register the pending request
        {
            let mut pending = self.pending_requests.write().await;
            pending.insert(
                id.clone(),
                PendingRequest {
                    sender: response_sender,
                },
            );
        }

        // Send the request
        {
            let mut transport = self.transport.write().await;
            transport
                .write_rpc_message(&RpcMessage::Request(request))
                .await?;
        }

        // Wait for the response
        match response_receiver.await {
            Ok(response) => Ok(response),
            Err(_) => {
                // Clean up the pending request if it wasn't already removed
                let mut pending = self.pending_requests.write().await;
                pending.remove(&id);
                Err(LspError::Other("Response receiver dropped".to_string()))
            }
        }
    }

    /// Send a notification (no response expected).
    pub async fn send_notification(
        &self,
        method: impl Into<String>,
        params: Option<serde_json::Value>,
    ) -> Result<()> {
        let notification = match params {
            Some(params) => NotificationMessage::with_params(method, params),
            None => NotificationMessage::new(method),
        };

        let mut transport = self.transport.write().await;
        transport
            .write_rpc_message(&RpcMessage::Notification(notification))
            .await
    }

    /// Receive the next incoming message (request or notification from server).
    /// This method should be called in a loop to handle all incoming messages.
    pub async fn receive_message(&mut self) -> Option<RpcMessage> {
        if let Some(ref mut receiver) = self.message_receiver {
            receiver.recv().await
        } else {
            None
        }
    }

    /// Send a response to a request from the server.
    pub async fn send_response(
        &self,
        id: Id,
        result: Option<serde_json::Value>,
        error: Option<crate::error::ResponseError>,
    ) -> Result<()> {
        let response = if let Some(error) = error {
            ResponseMessage::error(Some(id), error)
        } else {
            ResponseMessage::success(id, result.unwrap_or(serde_json::Value::Null))
        };

        let mut transport = self.transport.write().await;
        transport
            .write_rpc_message(&RpcMessage::Response(response))
            .await
    }

    /// Check if there are any pending requests.
    pub async fn has_pending_requests(&self) -> bool {
        !self.pending_requests.read().await.is_empty()
    }

    /// Get the number of pending requests.
    pub async fn pending_request_count(&self) -> usize {
        self.pending_requests.read().await.len()
    }

    /// Cancel all pending requests.
    pub async fn cancel_all_requests(&self) {
        let mut pending = self.pending_requests.write().await;
        pending.clear();
    }

    /// Initialize the LSP server with the given parameters.
    /// This is typically the first method called after creating the client.
    pub async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let response = self
            .send_request("initialize", Some(serde_json::to_value(params)?))
            .await?;

        if let Some(error) = response.error {
            return Err(LspError::InitializationFailed(format!(
                "Initialize request failed: {}",
                error.message
            )));
        }

        if let Some(result) = response.result {
            Ok(serde_json::from_value(result)?)
        } else {
            Err(LspError::InitializationFailed(
                "Initialize response missing result".to_string(),
            ))
        }
    }

    /// Send the 'initialized' notification to the server.
    /// This should be called after a successful 'initialize' request.
    pub async fn initialized(&self) -> Result<()> {
        self.send_notification("initialized", Some(serde_json::json!({})))
            .await
    }

    /// Complete the initialization handshake with default parameters.
    /// This is a convenience method that creates default initialization parameters
    /// and sends both the initialize request and initialized notification.
    pub async fn initialize_default(
        &self,
        client_name: impl Into<String>,
        client_version: Option<String>,
        root_uri: Option<String>,
    ) -> Result<InitializeResult> {
        let params = InitializeParams {
            process_id: Some(std::process::id()),
            client_info: Some(ClientInfo {
                name: client_name.into(),
                version: client_version,
            }),
            locale: None,
            root_path: None,
            root_uri,
            initialization_options: None,
            capabilities: ClientCapabilities::default(),
            trace: None,
            workspace_folders: None,
        };

        let result = self.initialize(params).await?;
        self.initialized().await?;
        Ok(result)
    }
}

/// Enum representing any type of JSON-RPC message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RpcMessage {
    // Order matters for untagged deserialization!
    // Most specific variants should come first
    Request(RequestMessage),           // Has both id and method
    Notification(NotificationMessage), // Has method, no id
    Response(ResponseMessage),         // May have only id, or no required fields
}

impl RpcMessage {
    /// Check if this is a request message.
    pub fn is_request(&self) -> bool {
        matches!(self, RpcMessage::Request(_))
    }

    /// Check if this is a response message.
    pub fn is_response(&self) -> bool {
        matches!(self, RpcMessage::Response(_))
    }

    /// Check if this is a notification message.
    pub fn is_notification(&self) -> bool {
        matches!(self, RpcMessage::Notification(_))
    }

    /// Get the method name if this is a request or notification.
    pub fn method(&self) -> Option<&str> {
        match self {
            RpcMessage::Request(req) => Some(&req.method),
            RpcMessage::Notification(notif) => Some(&notif.method),
            RpcMessage::Response(_) => None,
        }
    }

    /// Get the ID if this is a request or response.
    pub fn id(&self) -> Option<&Id> {
        match self {
            RpcMessage::Request(req) => Some(&req.id),
            RpcMessage::Response(resp) => resp.id.as_ref(),
            RpcMessage::Notification(_) => None,
        }
    }
}

/// The default content type for LSP messages.
pub const DEFAULT_CONTENT_TYPE: &str = "application/vscode-jsonrpc; charset=utf-8";

pub struct Reader {
    reader: BufReader<ChildStdout>,
}

impl Reader {
    pub fn new(reader: BufReader<ChildStdout>) -> Self {
        Self { reader }
    }

    /// Read a complete message from the transport.
    pub async fn read_message(&mut self) -> anyhow::Result<Message> {
        let headers = self.read_headers().await?;
        let content = self.read_content(&headers).await?;

        Ok(Message { headers, content })
    }

    /// Read message headers from the transport.
    async fn read_headers(&mut self) -> anyhow::Result<MessageHeaders> {
        let mut headers = HashMap::new();
        let mut content_length = None;
        let mut content_type = DEFAULT_CONTENT_TYPE.to_string();

        loop {
            let line = self.read_line().await?;

            // Empty line indicates end of headers
            if line.is_empty() {
                break;
            }

            // Parse header field
            if let Some((name, value)) = parse_header_field(&line)? {
                match name.to_lowercase().as_str() {
                    "content-length" => {
                        content_length = Some(
                            value
                                .parse::<usize>()
                                .map_err(|_| anyhow!("Invalid Content-Length: {}", value))?,
                        );
                    }
                    "content-type" => {
                        content_type = value;
                    }
                    _ => {
                        headers.insert(name, value);
                    }
                }
            }
        }

        let content_length =
            content_length.ok_or_else(|| anyhow!("Missing Content-Length header".to_string()))?;

        Ok(MessageHeaders {
            content_length,
            content_type,
            additional: headers,
        })
    }

    /// Read message content based on the headers.
    async fn read_content(&mut self, headers: &MessageHeaders) -> anyhow::Result<String> {
        let mut buffer = vec![0; headers.content_length];
        self.reader.read_exact(&mut buffer).await?;

        // Validate encoding
        let encoding = headers.get_encoding();
        if encoding != "utf-8" {
            return Err(anyhow!(format!("Unsupported encoding: {}", encoding)));
        }

        // Convert to string
        String::from_utf8(buffer).map_err(|e| anyhow!(format!("Invalid UTF-8 content: {}", e)))
    }

    /// Read a single line (ending with \r\n) from the transport.
    async fn read_line(&mut self) -> anyhow::Result<String> {
        let mut line = Vec::new();
        let mut prev_byte = 0u8;

        loop {
            let mut byte = [0u8; 1];
            self.reader.read_exact(&mut byte).await?;
            let byte = byte[0];

            if byte == b'\n' && prev_byte == b'\r' {
                // Remove the \r\n
                line.pop();
                break;
            }

            line.push(byte);
            prev_byte = byte;
        }

        String::from_utf8(line).map_err(|e| anyhow!(format!("Invalid UTF-8 in header: {}", e)))
    }
}

impl Writer {
    /// Write a message to the transport.
    pub async fn write_message(&mut self, message: &Message) -> anyhow::Result<()> {
        let bytes = message.to_bytes();
        self.writer.write_all(&bytes).await?;
        self.writer.flush().await?;
        Ok(())
    }

    /// Write an RPC message to the transport.
    pub async fn write_rpc_message(&mut self, rpc_message: &RpcMessage) -> anyhow::Result<()> {
        let message = Message::from_rpc_message(rpc_message)?;
        self.write_message(&message).await
    }
}

/// Parse a header field line into name and value.
fn parse_header_field(line: &str) -> anyhow::Result<Option<(String, String)>> {
    if line.is_empty() {
        return Ok(None);
    }

    if let Some((name, value)) = line.split_once(": ") {
        Ok(Some((name.trim().to_string(), value.trim().to_string())))
    } else {
        Err(anyhow!("Invalid header field: {}", line))
    }
}

/// Header fields for LSP messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageHeaders {
    /// The length of the content part in bytes.
    pub content_length: usize,
    /// The MIME type of the content part.
    pub content_type: String,
    /// Additional header fields.
    pub additional: HashMap<String, String>,
}

impl MessageHeaders {
    /// Create new headers with the given content length.
    pub fn new(content_length: usize) -> Self {
        Self {
            content_length,
            content_type: DEFAULT_CONTENT_TYPE.to_string(),
            additional: HashMap::new(),
        }
    }

    /// Create headers with custom content type.
    pub fn with_content_type(content_length: usize, content_type: impl Into<String>) -> Self {
        Self {
            content_length,
            content_type: content_type.into(),
            additional: HashMap::new(),
        }
    }

    /// Add an additional header field.
    pub fn add_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.additional.insert(name.into(), value.into());
        self
    }

    /// Get the character encoding from the content type.
    /// Returns "utf-8" by default, and also accepts "utf8" for backwards compatibility.
    pub fn get_encoding(&self) -> &str {
        if self.content_type.contains("charset=") {
            if let Some(charset_part) = self.content_type.split("charset=").nth(1) {
                let charset = charset_part.split(';').next().unwrap_or("utf-8").trim();
                // Handle backwards compatibility with "utf8"
                if charset == "utf8" {
                    return "utf-8";
                }
                return charset;
            }
        }
        "utf-8"
    }
}

/// A complete LSP message with headers and content.
#[derive(Debug, Clone)]
pub struct Message {
    pub headers: MessageHeaders,
    pub content: String,
}

impl Message {
    /// Create a new message with the given content.
    pub fn new(content: impl Into<String>) -> Self {
        let content = content.into();
        let content_bytes = content.len();

        Self {
            headers: MessageHeaders::new(content_bytes),
            content,
        }
    }

    /// Create a message from an RPC message by serializing it to JSON.
    pub fn from_rpc_message(rpc_message: &RpcMessage) -> Result<Self> {
        let content = serde_json::to_string(rpc_message)?;
        Ok(Self::new(content))
    }

    /// Parse the content as an RPC message.
    pub fn parse_rpc_message(&self) -> Result<RpcMessage> {
        Ok(serde_json::from_str(&self.content)?)
    }

    /// Serialize this message to bytes for transmission.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut result = Vec::new();

        // Add Content-Length header
        result.extend_from_slice(
            format!("Content-Length: {}\r\n", self.headers.content_length).as_bytes(),
        );

        // Add Content-Type header if not default
        if self.headers.content_type != DEFAULT_CONTENT_TYPE {
            result.extend_from_slice(
                format!("Content-Type: {}\r\n", self.headers.content_type).as_bytes(),
            );
        }

        // Add additional headers
        for (name, value) in &self.headers.additional {
            result.extend_from_slice(format!("{}: {}\r\n", name, value).as_bytes());
        }

        // Add separator between headers and content
        result.extend_from_slice(b"\r\n");

        // Add content
        result.extend_from_slice(self.content.as_bytes());

        result
    }
}
