use std::fmt::Display;

use chrono::{DateTime, Local, NaiveDateTime, SecondsFormat, Utc};
use reqwest::Client;
use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;
use tracing::{Level, span};

const URL: &str = "https://api.opentransportdata.swiss/ojp20";

#[derive(Debug)]
pub enum RequestType {
    LocationInformation,
    Trip,
    StopEvent,
    Unknown,
}

#[derive(Debug, Error)]
pub enum RequestError {
    #[error("Missing authetification token")]
    MissingAuthToken,
    #[error("Missing location name")]
    MissingLocationName,
    #[error("Missing from and to ids")]
    MissingFromAndToId,
    #[error("Missing from id")]
    MissingFromId,
    #[error("Missing to id")]
    MissingToId,
    #[error("Unknown request type: must be LocationInformation, Trip, or StopEvent")]
    UnknownRequestType,
    #[error("Events type is not implemented")]
    EventsRequestTypeNotImplemented,
    #[error("Invalid number of results, got {0}, should be > 0.")]
    InvalidNumberResults(u32),
    #[error("Http request error: {0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error(
        "{0} is not a valid local time for the current timezone offset (falls in a DST gap or overlap)"
    )]
    InvalidLocalDateTime(NaiveDateTime),
}

impl TryFrom<RequestType> for String {
    type Error = RequestError;
    fn try_from(value: RequestType) -> Result<Self, Self::Error> {
        match value {
            RequestType::LocationInformation => Ok("OJPLocationInformationRequest".to_string()),
            RequestType::Trip => Ok("OJPTripRequest".to_string()),
            RequestType::StopEvent => Ok("OJPStopEventRequest".to_string()),
            RequestType::Unknown => Err(RequestError::UnknownRequestType),
        }
    }
}

#[derive(Debug)]
pub struct RequestBuilder {
    token: Option<SecretString>,
    date_time: DateTime<Utc>,
    request_type: RequestType,
    number_results: u32,
    from: Option<i32>,
    to: Option<i32>,
    name: Option<String>,
    requestor_ref: String,
}

impl RequestBuilder {
    pub fn try_new(date_time: NaiveDateTime) -> Result<Self, RequestError> {
        // We convert NaiveDateTime to Utc through Local (for the offset)
        // First we get the "now" local time (used for the offset)
        // and add it to the NaiveDateTime
        let local_date_time = date_time
            .and_local_timezone(*Local::now().offset())
            .single()
            .ok_or(RequestError::InvalidLocalDateTime(date_time))?;

        let date_time = local_date_time.to_utc();
        Ok(RequestBuilder {
            date_time,
            token: None,
            request_type: RequestType::Unknown,
            number_results: 0,
            from: None,
            to: None,
            name: None,
            requestor_ref: String::new(),
        })
    }

    pub fn set_from(mut self, from: i32) -> Self {
        self.from = Some(from);
        self
    }

    pub fn set_to(mut self, to: i32) -> Self {
        self.to = Some(to);
        self
    }

    pub fn set_token(mut self, token: SecretString) -> Self {
        self.token = Some(token);
        self
    }

    pub fn set_request_type(mut self, request_type: RequestType) -> Self {
        self.request_type = request_type;
        self
    }

    pub fn set_number_results(mut self, number_results: u32) -> Self {
        self.number_results = number_results;
        self
    }

    pub fn set_name(mut self, name: &str) -> Self {
        self.name = Some(name.to_string());
        self
    }

    pub fn set_requestor_ref(mut self, requestor_ref: &str) -> Self {
        self.requestor_ref = requestor_ref.to_string();
        self
    }

    pub fn try_request_body(&self) -> Result<String, RequestError> {
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let date_time = self.date_time.to_rfc3339_opts(SecondsFormat::Millis, true);

        let number_results = self.number_results;
        match self.request_type {
            RequestType::Unknown => Err(RequestError::UnknownRequestType),
            RequestType::LocationInformation => {
                if number_results == 0 {
                    return Err(RequestError::InvalidNumberResults(number_results));
                }
                let Some(name) = self.name.as_ref() else {
                    return Err(RequestError::MissingLocationName);
                };
                let requestor_ref = quick_xml::escape::escape(self.requestor_ref.as_str());
                let name = quick_xml::escape::escape(name.as_str());
                let req = format!(
"<?xml version=\"1.0\" encoding=\"UTF-8\"?>
                            <OJP xmlns=\"http://www.vdv.de/ojp\" xmlns:siri=\"http://www.siri.org.uk/siri\" version=\"2.0\">
                             	<OJPRequest>
                                    <siri:ServiceRequest>
                                        <siri:RequestTimestamp>{now}</siri:RequestTimestamp>
                                        <siri:RequestorRef>{requestor_ref}</siri:RequestorRef>
                                        <OJPLocationInformationRequest>
                                        <siri:RequestTimestamp>{now}</siri:RequestTimestamp>
                                        <siri:MessageIdentifier>LIR-1a</siri:MessageIdentifier>
                                        <InitialInput>
                                            <Name>{name}</Name>
                                        </InitialInput>
                                        <Restrictions>
                                            <Type>stop</Type>
                                            <NumberOfResults>{number_results}</NumberOfResults>
                                        </Restrictions>
                                    </OJPLocationInformationRequest>
                                    </siri:ServiceRequest>
                                </OJPRequest>
                            </OJP>");
                Ok(req)
            }
            RequestType::StopEvent => Err(RequestError::EventsRequestTypeNotImplemented),
            RequestType::Trip => {
                if number_results == 0 {
                    return Err(RequestError::InvalidNumberResults(number_results));
                }
                let (from, to) = match (self.from, self.to) {
                    (Some(from), Some(to)) => (from, to),
                    (None, None) => return Err(RequestError::MissingFromAndToId),
                    (Some(_), None) => return Err(RequestError::MissingToId),
                    (None, Some(_)) => return Err(RequestError::MissingFromId),
                };
                let requestor_ref = quick_xml::escape::escape(self.requestor_ref.as_str());
                let req = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>
                            <OJP xmlns=\"http://www.vdv.de/ojp\" xmlns:siri=\"http://www.siri.org.uk/siri\" version=\"2.0\">
                             	<OJPRequest>
                                    <siri:ServiceRequest>
                                        <siri:RequestTimestamp>{now}</siri:RequestTimestamp>
                                        <siri:RequestorRef>{requestor_ref}</siri:RequestorRef>
                                        <OJPTripRequest>
                                            <siri:RequestTimestamp>{now}</siri:RequestTimestamp>
                                            <siri:MessageIdentifier>TR-1r1</siri:MessageIdentifier>
                                            <Origin>
                                                <PlaceRef>
                                                    <siri:StopPointRef>{from}</siri:StopPointRef>
                                                </PlaceRef>
                                                <DepArrTime>{date_time}</DepArrTime>
                                            </Origin>
                                            <Destination>
                                                <PlaceRef>
                                                    <siri:StopPointRef>{to}</siri:StopPointRef>
                                                </PlaceRef>
                                            </Destination>
                                            <Params>
                                                <NumberOfResults>{number_results}</NumberOfResults>
                                            </Params>
                                        </OJPTripRequest>
                                    </siri:ServiceRequest>
                                </OJPRequest>
                            </OJP>");
                Ok(req)
            }
        }
    }

    pub fn build_request(self) -> Result<reqwest::RequestBuilder, RequestError> {
        let id_request = self.try_request_body()?;

        if self.token.is_none() {
            return Err(RequestError::MissingAuthToken);
        }
        {
            let span = span!(Level::INFO, "Performing OJP request");
            let _guard = span.enter();
            tracing::info!("{}", self);
        }
        let token = self.token.ok_or(RequestError::MissingAuthToken)?;

        let req = Client::new()
            .post(URL)
            .header("Content-Type", "application/xml")
            .header("accept", "*/*")
            .bearer_auth(token.expose_secret())
            .body(id_request);

        Ok(req)
    }

    pub async fn send_request(self) -> Result<String, RequestError> {
        let respone = self.build_request()?.send().await?.text().await?;
        Ok(respone)
    }
}

impl Display for RequestBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let token = self
            .token
            .as_ref()
            .map(|s| format!("{:?}", s))
            .unwrap_or("Undefined".to_string());

        match self.request_type {
            RequestType::LocationInformation => {
                write!(f, "Location Information Request: ")?;
                write!(
                    f,
                    "Location: {}, ",
                    self.name
                        .as_ref()
                        .map(|i| i.to_string())
                        .unwrap_or("Undefined".to_string()),
                )?;
            }
            RequestType::Trip => {
                write!(f, "Trip Request: ")?;
                write!(
                    f,
                    "From: {}, To: {}, ",
                    self.from
                        .map(|i| format!("{i}"))
                        .unwrap_or("Undefined".to_string()),
                    self.to
                        .map(|i| format!("{i}"))
                        .unwrap_or("Undefined".to_string()),
                )?;
            }
            RequestType::StopEvent => {
                write!(f, "Stop Envent Request Not Implement. ")?;
            }
            RequestType::Unknown => {
                write!(f, "RequestType is unknown. ")?;
            }
        }
        write!(
            f,
            "NumberResults: {}, DateTime: {}, RequestorRef: {}, Token: {token}",
            self.number_results, self.date_time, self.requestor_ref
        )
    }
}
