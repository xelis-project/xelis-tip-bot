use teloxide::{payloads::{SendMessage, SendMessageSetters}, prelude::Requester, requests::JsonRequest, types::{ChatId, ParseMode}, Bot};

pub struct TelegramMessage<'a> {
    title: Option<String>,
    lines: Vec<String>,
    bot: &'a Bot,
    chat_id: ChatId
}

pub struct InlineCode<'a> {
    text: &'a str
}

impl<'a> InlineCode<'a> {
    pub fn new(text: &'a str) -> Self {
        InlineCode { text }
    }
}

impl ToString for InlineCode<'_> {
    fn to_string(&self) -> String {
        format!("<code>{}</code>", self.text)
    }
}

impl Into<String> for InlineCode<'_> {
    fn into(self) -> String {
        self.to_string()
    }
}

const NEW_LINE: &str = "\n";

impl<'a> TelegramMessage<'a> {
    pub fn new(bot: &'a Bot, chat_id: ChatId) -> Self {
        TelegramMessage {
            title: None,
            lines: Vec::new(),
            bot,
            chat_id
        }
    }

    pub fn title(&mut self, text: &str) -> &mut Self {
        self.title = Some(format!("<strong>{}</strong>", text));
        self
    }

    pub fn field<S: Into<String>>(&mut self, text: &str, value: S, inline: bool) -> &mut Self {
        self.lines.push(format!("<strong>{}</strong>{}{}", text, if inline { " " } else { NEW_LINE }, value.into()));
        self
    }

    pub fn to_string(&self) -> String {
        let mut buf = String::new();
        if let Some(title) = &self.title {
            buf.push_str(&title);
            if !self.lines.is_empty() {
                buf.push_str(NEW_LINE);
                buf.push_str(NEW_LINE);
            }
        }

        for line in self.lines.iter() {
            buf.push_str(NEW_LINE);
            buf.push_str(&line);
            buf.push_str(NEW_LINE);
        }

        buf
    }

    pub fn send(&self) -> JsonRequest<SendMessage> {
        self.bot.send_message(self.chat_id, self.to_string())
            .parse_mode(ParseMode::Html)
    }
}