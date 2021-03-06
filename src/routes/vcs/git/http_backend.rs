use std::path::{Path, PathBuf};

use rocket::{
    data::ByteUnit,
    handler::{Handler, Outcome},
    http::Method,
    Config, Data, Request, Response, Route, State,
};

use crate::cgi::{CgiScript, CgiScriptError};

#[derive(Clone, Debug)]
pub struct GitHttpBackend {
    repo_dir: PathBuf,
}

impl GitHttpBackend {
    pub fn new<P: AsRef<Path>>(repo_dir: P) -> Self {
        let repo_dir = repo_dir.as_ref().to_path_buf();
        Self { repo_dir }
    }
}

#[async_trait::async_trait]
impl Handler for GitHttpBackend {
    async fn handle<'r, 's: 'r>(&'s self, request: &'r Request<'_>, data: Data) -> Outcome<'r> {
        // TODO: Handle the error case.
        let config: State<Config> = request.guard().await.unwrap();

        let mut request_path = request.uri().path().to_string();
        if !request_path.ends_with(".git") {
            request_path.push_str(".git");
        }

        let data = {
            let mut data = data.open(ByteUnit::max_value());
            let mut buf = Vec::new();
            tokio::io::copy(&mut data, &mut buf).await.map(|_| buf)
        };
        let response = data
            .map_err(|err| CgiScriptError::Io(err))
            .and_then(|data| {
                CgiScript::new("git", &["http-backend"], &[])
                    .server_software("rocket")
                    .server_name(&config.address.to_string())
                    .server_port(&config.port.to_string())
                    .request_method(request.method().as_str())
                    .query_string(request.uri().query().unwrap_or(""))
                    .remote_addr(
                        &request
                            .client_ip()
                            .map(|ip| ip.to_string())
                            .unwrap_or_default(),
                    )
                    .path_info(&request_path)
                    .path_translated(&translate_git_path(&self.repo_dir, request))
                    .content_type(
                        &request
                            .content_type()
                            .map(|ct| ct.to_string())
                            .unwrap_or_default(),
                    )
                    .run(data.as_slice())
                    .map(|response| {
                        let response: Response = response.into();
                        response
                    })
            });

        Outcome::try_from(request, response)
    }
}

fn translate_git_path(repo_dir: &Path, request: &Request) -> String {
    repo_dir
        .join(
            request
                .uri()
                .segments()
                .enumerate()
                .fold(String::new(), |mut path, (i, segment)| {
                    if i > 0 {
                        #[cfg(windows)]
                        path.push('\\');
                        #[cfg(not(windows))]
                        path.push('/');
                    }
                    path.push_str(segment);
                    if i == 1 && !path.ends_with(".git") {
                        path.push_str(".git");
                    }
                    path
                }),
        )
        .to_str()
        .unwrap()
        .replace('\\', "/")
}

impl Into<Vec<Route>> for GitHttpBackend {
    fn into(self) -> Vec<Route> {
        vec![
            Route::new(Method::Get, "/<user>/<repo>/info/refs", self.clone()),
            Route::new(Method::Post, "/<user>/<repo>/git-upload-pack", self.clone()),
            Route::new(
                Method::Post,
                "/<user>/<repo>/git-receive-pack",
                self.clone(),
            ),
        ]
    }
}
