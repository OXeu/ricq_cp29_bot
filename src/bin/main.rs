#![feature(async_closure)]

use std::fs;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    routing::{get, get_service, post},
    Extension, Router,
};
use chrono::NaiveDateTime;
use dashmap::DashMap;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tower::ServiceBuilder;
use tower_http::services::ServeDir;
use tracing::Level;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use ricq::client::{DefaultConnector, NetworkStatus};
use ricq::ext::common::after_login;
use ricq::ext::reconnect::{auto_reconnect, Credential};
use ricq::handler::QEvent;
use ricq::Client;
use ricq::{
    client::event::{FriendMessageEvent, GroupMessageEvent},
    msg::{
        elem::{At, Text},
        MessageChain,
    },
};
use ricq_axum_api::handler::{bot, password, qrcode};
use ricq_axum_api::processor::Processor;
use ricq_axum_api::u8_protocol::U8Protocol;
use ricq_axum_api::{ClientInfo, RicqAxumApi};
const QQ: i64 = 797443771; // 发送对象
                           // 默认处理器
struct ClientProcessor(DashMap<(i64, u8), Arc<Client>>);

#[async_trait::async_trait]
impl Processor for ClientProcessor {
    async fn on_login_success(
        &self,
        client: Arc<Client>,
        mut event_receiver: broadcast::Receiver<QEvent>,
        credential: Credential,
        network_join_handle: JoinHandle<()>,
    ) {
        let uin = client.uin().await;
        let protocol = client.version().await.protocol.to_u8();
        self.0.insert((uin, protocol), client.clone());
        after_login(&client).await;

        tokio::spawn(async move {
            while let Ok(event) = event_receiver.recv().await {
                match event {
                    QEvent::GroupMessage(e) => {
                        let GroupMessageEvent {
                            inner: message,
                            client: _,
                        } = e;
                        tracing::info!(
                            "GROUP_MSG, code: {}, content: {}",
                            message.group_code,
                            message.elements.to_string()
                        );
                        // client
                        //     .send_group_message(message.group_code, message.elements)
                        //     .await
                        //     .ok();
                    }
                    QEvent::FriendMessage(e) => {
                        let FriendMessageEvent {
                            inner: message,
                            client,
                        } = e;
                        tracing::info!(
                            "FRIEND_MSG, code: {}, content: {}",
                            message.from_uin,
                            message.elements.to_string()
                        );
                        if message.from_uin == 1573856599 {
                            let mut chain = MessageChain::default();
                            chain.push(Text::new(String::from("Hello")));
                            client
                                .send_friend_message(message.from_uin, chain)
                                .await
                                .ok();
                        }
                    }
                    other => {
                        tracing::info!("{:?}", other)
                    }
                }
            }
        });
        let c1 = client.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(10)).await;
                // c1.send_friend_message(1573856599, chain).await.ok();
                let data: Result<serde_json::Value, reqwest::Error> = reqwest::get("https://www.allcpp.cn/allcpp/eventMain/notice/getList.do?id=1074&pageindex=1&pagesize=1")
                    .await
                    .unwrap()
                    .json()
                    .await;
                if let Ok(dat) = data {
                    let v = dat.as_array().unwrap().get(0).unwrap();
                    let works = v["works"].clone();
                    let id = works["id"].as_i64().unwrap();
                    let title = works["name"].as_str().unwrap();
                    let foreword = works["foreword"].as_str().unwrap();
                    let content = works["content"].as_str().unwrap();
                    let time = v["createTime"].as_i64().unwrap();
                    let date_time =
                        NaiveDateTime::from_timestamp_opt((time + 8 * 3600_000) / 1000, 0).unwrap();
                    let mut chain = MessageChain::default();
                    if content.contains("票") {
                        chain.push(At::new(3040692186));
                        chain.push(At::new(1573856599));
                        chain.push(Text::new(format!(" 也许跟门票有关：\n\n{title}\n{foreword}\n\n活动详情:https://www.allcpp.cn/w/{id}.do\n\n发布时间:{date_time}")));
                    } else {
                        chain.push(Text::new(format!("{title}\n\n活动详情:https://www.allcpp.cn/w/{id}.do\n\n发布时间:{date_time}")));
                    }
                    match fs::read("last.txt") {
                        Ok(v) => {
                            let last = String::from_utf8(v).unwrap();
                            if last != time.to_string() {
                                c1.send_group_message(QQ, chain).await.ok();
                                fs::write("last.txt", time.to_string()).unwrap();
                            }
                        }
                        Err(_) => {
                            c1.send_friend_message(QQ, chain).await.ok();
                            fs::write("last.txt", time.to_string()).unwrap();
                        }
                    }
                }
            }
        });

        let c = client.clone();
        tokio::spawn(async {
            network_join_handle.await.ok();
            auto_reconnect(c, credential, Duration::from_secs(10), 10, DefaultConnector).await;
        });

        // DONT BLOCK
    }

    async fn list_client(&self) -> Vec<ClientInfo> {
        let mut infos = Vec::new();
        for cli in self.0.iter() {
            let (uin, protocol) = cli.key();
            let client = cli.value();
            infos.push(ClientInfo {
                uin: *uin,
                nick: client.account_info.read().await.nickname.clone(),
                status: client.get_status(),
                protocol: *protocol,
            });
        }
        infos
    }

    async fn delete_client(&self, uin: i64, protocol: u8) {
        if let Some((_, client)) = self.0.remove(&(uin, protocol)) {
            client.stop(NetworkStatus::Stop);
        }
    }
}

#[tokio::main]
async fn main() {
    // 初始化日志
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(true)
                .with_timer(tracing_subscriber::fmt::time::OffsetTime::new(
                    time::UtcOffset::__from_hms_unchecked(8, 0, 0),
                    time::macros::format_description!(
                        "[year repr:last_two]-[month]-[day] [hour]:[minute]:[second]"
                    ),
                )),
        )
        .with(
            tracing_subscriber::filter::Targets::new()
                .with_target("main", Level::DEBUG)
                .with_target("ricq", Level::DEBUG)
                .with_target("ricq_axum_api", Level::DEBUG),
        )
        .init();

    let processor = ClientProcessor(Default::default());
    let ricq_axum_api = Arc::new(RicqAxumApi::new(processor));

    let app = Router::new()
        .route("/ping", get(async move || "pong"))
        .nest(
            "/login",
            Router::new()
                .nest(
                    "/qrcode",
                    Router::new()
                        .route("/create", post(qrcode::create::<ClientProcessor>))
                        .route("/list", get(qrcode::list::<ClientProcessor>))
                        .route("/delete", post(qrcode::delete::<ClientProcessor>))
                        .route("/query", post(qrcode::query::<ClientProcessor>)),
                )
                .nest(
                    "/password",
                    Router::new()
                        .route("/create", post(password::login::<ClientProcessor>))
                        .route(
                            "/request_sms",
                            post(password::request_sms::<ClientProcessor>),
                        )
                        .route("/submit_sms", post(password::submit_sms::<ClientProcessor>))
                        .route(
                            "/submit_ticket",
                            post(password::submit_ticket::<ClientProcessor>),
                        )
                        .route("/list", get(password::list::<ClientProcessor>))
                        .route("/delete", post(password::delete::<ClientProcessor>)),
                ),
        )
        .nest(
            "/bot",
            Router::new()
                .route("/list", get(bot::list::<ClientProcessor>))
                .route("/delete", post(bot::delete::<ClientProcessor>)),
        )
        .fallback(get_service(ServeDir::new("static")).handle_error(handle_error))
        .layer(
            ServiceBuilder::new()
                .layer(Extension(ricq_axum_api))
                .into_inner(),
        );
    let addr = SocketAddr::from_str("0.0.0.0:9000").expect("failed to parse bind_addr");
    println!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn handle_error(_: std::io::Error) -> impl axum::response::IntoResponse {
    (axum::http::StatusCode::NOT_FOUND, "Something went wrong...")
}

/*
{
  "id": 2109,
  "worksId": 4576832,
  "works": {
    "id": 4576832,
    "name": "CP29【时空中的绘旅人】专区成立",
    "foreword": "——山海相逢，不渝之遇。",
    "type": 1,
    "interId": 35,
    "inter": "大众",
    "produceType": 1,
    "theme": "时空中的绘旅人",
    "userId": 51321,
    "user": {
      "id": 51321,
      "nickname": "Comicup组委会",
      "face": {
        "userId": 51321,
        "picUrl": "/2023/3/228ca757-b50e-4c75-958d-7cc21d5f9acb",
        "width": 494,
        "height": 494,
        "size": 309922
      },
      "background": null,
      "accountStatus": 1,
      "accountLevel": 2,
      "objCount": null
    },
    "layoutStatus": 1,
    "orderStatus": 1,
    "amount": 0,
    "amountEntity": null,
    "checkStatus": 1,
    "isShowHotCount": true,
    "isDraft": null,
    "isChat": false,
    "pics": [
      {
        "picId": 2863613,
        "sourcePosition": 0,
        "pic": {
          "userId": 51321,
          "picUrl": "/2023/4/411afd7c-1977-4a70-a87e-1c12b727984a.png",
          "width": 911,
          "height": 309,
          "size": 332946
        }
      }
    ],
    "tags": [
      {
        "tag": "时空中的绘旅人"
      }
    ],
    "content": "<p style=\"margin-bottom: 24px;\"><a href=\"https://www.allcpp.cn/allcpp/event/event.do?event=1074\" rel=\"noopener noreferrer\" target=\"_blank\">CP29</a>&nbsp;-倒计时25天-</p><p style=\"margin-bottom: 24px;\">【<a href=\"https://cp.allcpp.cn/#/tag/pic?tag=%E6%97%B6%E7%A9%BA%E4%B8%AD%E7%9A%84%E7%BB%98%E6%97%85%E4%BA%BA\" rel=\"noopener noreferrer\" target=\"_blank\">时空中的绘旅人</a>】专区</p><p style=\"margin-bottom: 24px;\"><br></p><p style=\"margin-bottom: 24px;\">——山海相逢，不渝之遇。</p><p style=\"margin-bottom: 24px;\"><br></p><p style=\"margin-bottom: 24px;\">【宣图感谢】<a href=\"https://weibo.com/n/%E9%9B%AA%E4%BB%A3%E8%96%B0\" rel=\"noopener noreferrer\" target=\"_blank\">@雪代薰</a></p><p style=\"margin-bottom: 24px;\">【专区街委】<a href=\"https://weibo.com/n/%E8%A7%A3%E6%A2%A6%E5%86%8D%E9%80%A0\" rel=\"noopener noreferrer\" target=\"_blank\">@解梦再造</a></p><p style=\"margin-bottom: 24px;\">【摊主群】600853334</p><p style=\"margin-bottom: 24px;\">【游客群】996869813</p><p style=\"margin-bottom: 24px;\"><br></p><p style=\"margin-bottom: 24px;\">【CP29✦活动主页✦】→<a href=\"https://www.allcpp.cn/allcpp/event/event.do?event=1074\" rel=\"noopener noreferrer\" target=\"_blank\">www.allcpp.cn/allcpp/event/event.do?event=1074</a></p><p style=\"margin-bottom: 24px;\">↑点击“我想去”，开票前即可收到提醒哦↑</p><p style=\"margin-bottom: 24px;\"><img src=\"https://imagecdn3.allcpp.cn/upload/2023/4/e8827627-5fb5-402f-9f2e-232d39390dd5.jpg?x-oss-process=style/format_jpg\"></p>",
    "openStatus": {
      "readOpenStatus": 2,
      "updateOpenStatus": 2,
      "deleteOpenStatus": 2,
      "linkOpenStatus": null
    },
    "roleCanRead": {
      "fansCanRead": false,
      "friendCanRead": false,
      "circleFansCanRead": false,
      "circleNormalMemberCanRead": false,
      "circleAdminMemberCanRead": false
    },
    "isReward": false,
    "payStatus": 0,
    "hotCount": 178,
    "isCollect": false,
    "isRecommend": false,
    "optionStatement": 2,
    "comicType": 0,
    "waiver": "",
    "flyleaf": null,
    "rpsSwitch": false,
    "isOnePayed": false,
    "createTime": 1680799575000,
    "updateTime": 1680799575000
  },
  "userId": 51321,
  "priority": 0,
  "chapterNo": null,
  "createTime": 1680799758000
}
 */
