use std::net::SocketAddr;
use std::path::Path;

/// Generate the UPnP device description XML.
pub fn device_xml(device_uuid: &str, folder_name: &str, addr: SocketAddr) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns="urn:schemas-upnp-org:device-1-0">
  <specVersion><major>1</major><minor>0</minor></specVersion>
  <device>
    <deviceType>urn:schemas-upnp-org:device:MediaServer:1</deviceType>
    <friendlyName>Looky — {folder_name}</friendlyName>
    <manufacturer>Looky</manufacturer>
    <modelName>Looky Photo Server</modelName>
    <UDN>uuid:{device_uuid}</UDN>
    <serviceList>
      <service>
        <serviceType>urn:schemas-upnp-org:service:ContentDirectory:1</serviceType>
        <serviceId>urn:upnp-org:serviceId:ContentDirectory</serviceId>
        <SCPDURL>/dlna/content.xml</SCPDURL>
        <controlURL>/dlna/control/content</controlURL>
        <eventSubURL>/dlna/event/content</eventSubURL>
      </service>
      <service>
        <serviceType>urn:schemas-upnp-org:service:ConnectionManager:1</serviceType>
        <serviceId>urn:upnp-org:serviceId:ConnectionManager</serviceId>
        <SCPDURL>/dlna/connection.xml</SCPDURL>
        <controlURL>/dlna/control/connection</controlURL>
        <eventSubURL>/dlna/event/connection</eventSubURL>
      </service>
    </serviceList>
    <presentationURL>http://{addr}/</presentationURL>
  </device>
</root>"#
    )
}

/// ContentDirectory SCPD (service description).
pub fn content_directory_scpd() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<scpd xmlns="urn:schemas-upnp-org:service-1-0">
  <specVersion><major>1</major><minor>0</minor></specVersion>
  <actionList>
    <action>
      <name>Browse</name>
      <argumentList>
        <argument><name>ObjectID</name><direction>in</direction><relatedStateVariable>A_ARG_TYPE_ObjectID</relatedStateVariable></argument>
        <argument><name>BrowseFlag</name><direction>in</direction><relatedStateVariable>A_ARG_TYPE_BrowseFlag</relatedStateVariable></argument>
        <argument><name>Filter</name><direction>in</direction><relatedStateVariable>A_ARG_TYPE_Filter</relatedStateVariable></argument>
        <argument><name>StartingIndex</name><direction>in</direction><relatedStateVariable>A_ARG_TYPE_Index</relatedStateVariable></argument>
        <argument><name>RequestedCount</name><direction>in</direction><relatedStateVariable>A_ARG_TYPE_Count</relatedStateVariable></argument>
        <argument><name>SortCriteria</name><direction>in</direction><relatedStateVariable>A_ARG_TYPE_SortCriteria</relatedStateVariable></argument>
        <argument><name>Result</name><direction>out</direction><relatedStateVariable>A_ARG_TYPE_Result</relatedStateVariable></argument>
        <argument><name>NumberReturned</name><direction>out</direction><relatedStateVariable>A_ARG_TYPE_Count</relatedStateVariable></argument>
        <argument><name>TotalMatches</name><direction>out</direction><relatedStateVariable>A_ARG_TYPE_Count</relatedStateVariable></argument>
        <argument><name>UpdateID</name><direction>out</direction><relatedStateVariable>A_ARG_TYPE_UpdateID</relatedStateVariable></argument>
      </argumentList>
    </action>
    <action>
      <name>GetSystemUpdateID</name>
      <argumentList>
        <argument><name>Id</name><direction>out</direction><relatedStateVariable>SystemUpdateID</relatedStateVariable></argument>
      </argumentList>
    </action>
    <action>
      <name>GetSearchCapabilities</name>
      <argumentList>
        <argument><name>SearchCaps</name><direction>out</direction><relatedStateVariable>SearchCapabilities</relatedStateVariable></argument>
      </argumentList>
    </action>
    <action>
      <name>GetSortCapabilities</name>
      <argumentList>
        <argument><name>SortCaps</name><direction>out</direction><relatedStateVariable>SortCapabilities</relatedStateVariable></argument>
      </argumentList>
    </action>
  </actionList>
  <serviceStateTable>
    <stateVariable sendEventsAttribute="no"><name>A_ARG_TYPE_ObjectID</name><dataType>string</dataType></stateVariable>
    <stateVariable sendEventsAttribute="no"><name>A_ARG_TYPE_Result</name><dataType>string</dataType></stateVariable>
    <stateVariable sendEventsAttribute="no"><name>A_ARG_TYPE_BrowseFlag</name><dataType>string</dataType><allowedValueList><allowedValue>BrowseMetadata</allowedValue><allowedValue>BrowseDirectChildren</allowedValue></allowedValueList></stateVariable>
    <stateVariable sendEventsAttribute="no"><name>A_ARG_TYPE_Filter</name><dataType>string</dataType></stateVariable>
    <stateVariable sendEventsAttribute="no"><name>A_ARG_TYPE_SortCriteria</name><dataType>string</dataType></stateVariable>
    <stateVariable sendEventsAttribute="no"><name>A_ARG_TYPE_Index</name><dataType>ui4</dataType></stateVariable>
    <stateVariable sendEventsAttribute="no"><name>A_ARG_TYPE_Count</name><dataType>ui4</dataType></stateVariable>
    <stateVariable sendEventsAttribute="no"><name>A_ARG_TYPE_UpdateID</name><dataType>ui4</dataType></stateVariable>
    <stateVariable sendEventsAttribute="yes"><name>SystemUpdateID</name><dataType>ui4</dataType></stateVariable>
    <stateVariable sendEventsAttribute="no"><name>SearchCapabilities</name><dataType>string</dataType></stateVariable>
    <stateVariable sendEventsAttribute="no"><name>SortCapabilities</name><dataType>string</dataType></stateVariable>
  </serviceStateTable>
</scpd>"#
}

/// ConnectionManager SCPD.
pub fn connection_manager_scpd() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<scpd xmlns="urn:schemas-upnp-org:service-1-0">
  <specVersion><major>1</major><minor>0</minor></specVersion>
  <actionList>
    <action>
      <name>GetProtocolInfo</name>
      <argumentList>
        <argument><name>Source</name><direction>out</direction><relatedStateVariable>SourceProtocolInfo</relatedStateVariable></argument>
        <argument><name>Sink</name><direction>out</direction><relatedStateVariable>SinkProtocolInfo</relatedStateVariable></argument>
      </argumentList>
    </action>
    <action>
      <name>GetCurrentConnectionIDs</name>
      <argumentList>
        <argument><name>ConnectionIDs</name><direction>out</direction><relatedStateVariable>CurrentConnectionIDs</relatedStateVariable></argument>
      </argumentList>
    </action>
    <action>
      <name>GetCurrentConnectionInfo</name>
      <argumentList>
        <argument><name>ConnectionID</name><direction>in</direction><relatedStateVariable>A_ARG_TYPE_ConnectionID</relatedStateVariable></argument>
        <argument><name>RcsID</name><direction>out</direction><relatedStateVariable>A_ARG_TYPE_RcsID</relatedStateVariable></argument>
        <argument><name>AVTransportID</name><direction>out</direction><relatedStateVariable>A_ARG_TYPE_AVTransportID</relatedStateVariable></argument>
        <argument><name>ProtocolInfo</name><direction>out</direction><relatedStateVariable>A_ARG_TYPE_ProtocolInfo</relatedStateVariable></argument>
        <argument><name>PeerConnectionManager</name><direction>out</direction><relatedStateVariable>A_ARG_TYPE_ConnectionManager</relatedStateVariable></argument>
        <argument><name>PeerConnectionID</name><direction>out</direction><relatedStateVariable>A_ARG_TYPE_ConnectionID</relatedStateVariable></argument>
        <argument><name>Direction</name><direction>out</direction><relatedStateVariable>A_ARG_TYPE_Direction</relatedStateVariable></argument>
        <argument><name>Status</name><direction>out</direction><relatedStateVariable>A_ARG_TYPE_ConnectionStatus</relatedStateVariable></argument>
      </argumentList>
    </action>
  </actionList>
  <serviceStateTable>
    <stateVariable sendEventsAttribute="yes"><name>SourceProtocolInfo</name><dataType>string</dataType></stateVariable>
    <stateVariable sendEventsAttribute="yes"><name>SinkProtocolInfo</name><dataType>string</dataType></stateVariable>
    <stateVariable sendEventsAttribute="yes"><name>CurrentConnectionIDs</name><dataType>string</dataType></stateVariable>
    <stateVariable sendEventsAttribute="no"><name>A_ARG_TYPE_ConnectionStatus</name><dataType>string</dataType></stateVariable>
    <stateVariable sendEventsAttribute="no"><name>A_ARG_TYPE_ConnectionManager</name><dataType>string</dataType></stateVariable>
    <stateVariable sendEventsAttribute="no"><name>A_ARG_TYPE_Direction</name><dataType>string</dataType></stateVariable>
    <stateVariable sendEventsAttribute="no"><name>A_ARG_TYPE_ProtocolInfo</name><dataType>string</dataType></stateVariable>
    <stateVariable sendEventsAttribute="no"><name>A_ARG_TYPE_ConnectionID</name><dataType>i4</dataType></stateVariable>
    <stateVariable sendEventsAttribute="no"><name>A_ARG_TYPE_AVTransportID</name><dataType>i4</dataType></stateVariable>
    <stateVariable sendEventsAttribute="no"><name>A_ARG_TYPE_RcsID</name><dataType>i4</dataType></stateVariable>
  </serviceStateTable>
</scpd>"#
}

/// Handle a SOAP action on ContentDirectory.
pub fn handle_content_directory(body: &str, addr: SocketAddr, image_paths: &[std::path::PathBuf]) -> String {
    let action = extract_soap_action(body);
    match action.as_deref() {
        Some("Browse") => handle_browse(body, addr, image_paths),
        Some("GetSystemUpdateID") => soap_response("GetSystemUpdateID", "<Id>1</Id>"),
        Some("GetSearchCapabilities") => soap_response("GetSearchCapabilities", "<SearchCaps></SearchCaps>"),
        Some("GetSortCapabilities") => soap_response("GetSortCapabilities", "<SortCaps></SortCaps>"),
        _ => soap_response("Browse", "<Result></Result><NumberReturned>0</NumberReturned><TotalMatches>0</TotalMatches><UpdateID>1</UpdateID>"),
    }
}

/// Handle a SOAP action on ConnectionManager.
pub fn handle_connection_manager(body: &str) -> String {
    let action = extract_soap_action(body);
    match action.as_deref() {
        Some("GetProtocolInfo") => soap_response(
            "GetProtocolInfo",
            "<Source>http-get:*:image/jpeg:*,http-get:*:image/png:*,http-get:*:image/gif:*,http-get:*:image/bmp:*,http-get:*:image/webp:*</Source><Sink></Sink>",
        ),
        Some("GetCurrentConnectionIDs") => {
            soap_response("GetCurrentConnectionIDs", "<ConnectionIDs>0</ConnectionIDs>")
        }
        Some("GetCurrentConnectionInfo") => soap_response(
            "GetCurrentConnectionInfo",
            "<RcsID>-1</RcsID><AVTransportID>-1</AVTransportID><ProtocolInfo></ProtocolInfo><PeerConnectionManager></PeerConnectionManager><PeerConnectionID>-1</PeerConnectionID><Direction>Output</Direction><Status>OK</Status>",
        ),
        _ => soap_response("GetProtocolInfo", "<Source></Source><Sink></Sink>"),
    }
}

fn extract_soap_action(body: &str) -> Option<String> {
    // Look for the action name in the SOAP body, e.g. <u:Browse ...> or soapaction header
    // Try to find <u:ActionName or <ActionName in the body
    for prefix in &["<u:", "<m:", "<"] {
        if let Some(start) = body.find(prefix) {
            let rest = &body[start + prefix.len()..];
            let end = rest.find(|c: char| c == ' ' || c == '>' || c == '/')?;
            let action = &rest[..end];
            // Skip known non-action tags
            if !matches!(action, "Envelope" | "Body" | "Header" | "s:Envelope" | "s:Body") {
                return Some(action.to_string());
            }
        }
    }
    None
}

fn handle_browse(body: &str, addr: SocketAddr, image_paths: &[std::path::PathBuf]) -> String {
    let object_id = extract_xml_value(body, "ObjectID").unwrap_or_else(|| "0".to_string());
    let browse_flag = extract_xml_value(body, "BrowseFlag").unwrap_or_else(|| "BrowseDirectChildren".to_string());
    let starting_index: usize = extract_xml_value(body, "StartingIndex")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let requested_count: usize = extract_xml_value(body, "RequestedCount")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let total = image_paths.len();

    if browse_flag == "BrowseMetadata" && object_id == "0" {
        // Root container metadata
        let didl = format!(
            r#"<DIDL-Lite xmlns="urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:upnp="urn:schemas-upnp-org:metadata-1-0/upnp/"><container id="0" parentID="-1" restricted="1" childCount="{total}"><dc:title>Photos</dc:title><upnp:class>object.container.storageFolder</upnp:class></container></DIDL-Lite>"#
        );
        let escaped = xml_escape(&didl);
        return soap_response(
            "Browse",
            &format!("<Result>{escaped}</Result><NumberReturned>1</NumberReturned><TotalMatches>1</TotalMatches><UpdateID>1</UpdateID>"),
        );
    }

    // BrowseDirectChildren of root
    let count = if requested_count == 0 { total } else { requested_count };
    let end = (starting_index + count).min(total);
    let slice = starting_index..end;
    let number_returned = slice.len();

    let mut didl_items = String::new();
    for i in slice {
        if let Some(path) = image_paths.get(i) {
            let title = xml_escape(&file_title(path));
            let mime = mime_for_path(path);
            let image_url = format!("http://{addr}/image/{i}");
            let thumb_url = format!("http://{addr}/thumb/{i}");
            didl_items.push_str(&format!(
                r#"<item id="{i}" parentID="0" restricted="1"><dc:title>{title}</dc:title><upnp:class>object.item.imageItem.photo</upnp:class><res protocolInfo="http-get:*:{mime}:*">{image_url}</res><upnp:albumArtURI>{thumb_url}</upnp:albumArtURI></item>"#
            ));
        }
    }

    let didl = format!(
        r#"<DIDL-Lite xmlns="urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:upnp="urn:schemas-upnp-org:metadata-1-0/upnp/">{didl_items}</DIDL-Lite>"#
    );
    let escaped = xml_escape(&didl);
    soap_response(
        "Browse",
        &format!("<Result>{escaped}</Result><NumberReturned>{number_returned}</NumberReturned><TotalMatches>{total}</TotalMatches><UpdateID>1</UpdateID>"),
    )
}

fn soap_response(action: &str, inner: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/" s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">
<s:Body><u:{action}Response xmlns:u="urn:schemas-upnp-org:service:ContentDirectory:1">{inner}</u:{action}Response></s:Body>
</s:Envelope>"#
    )
}

fn extract_xml_value(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].to_string())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn file_title(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Photo".to_string())
}

pub fn mime_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .as_deref()
    {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("bmp") => "image/bmp",
        Some("webp") => "image/webp",
        Some("tiff" | "tif") => "image/tiff",
        _ => "application/octet-stream",
    }
}
