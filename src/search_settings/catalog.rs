//! Provider metadata and independent search/read configurations.
use super::*;

// Keep the five existing positions stable for stored configurations and imports.
pub(super) const PROVIDERS: [Provider; 15] = [
    Provider::Tavily,
    Provider::Brave,
    Provider::Exa,
    Provider::Direct,
    Provider::Jina,
    Provider::Zhipu,
    Provider::Searxng,
    Provider::ExaMcp,
    Provider::Bocha,
    Provider::Querit,
    Provider::Parallel,
    Provider::JinaSearch,
    Provider::Firecrawl,
    Provider::QueritFetch,
    Provider::FirecrawlFetch,
];
pub(super) const SEARCH_PROVIDERS: [Provider; 11] = [
    Provider::Zhipu,
    Provider::Tavily,
    Provider::Searxng,
    Provider::Exa,
    Provider::ExaMcp,
    Provider::Bocha,
    Provider::Querit,
    Provider::Brave,
    Provider::Parallel,
    Provider::JinaSearch,
    Provider::Firecrawl,
];
pub(super) const FETCH_PROVIDERS: [Provider; 4] = [
    Provider::Jina,
    Provider::QueritFetch,
    Provider::FirecrawlFetch,
    Provider::Direct,
];

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub(super) enum Provider {
    Tavily,
    Brave,
    Exa,
    Direct,
    Jina,
    Zhipu,
    Searxng,
    #[serde(rename = "exa-mcp")]
    ExaMcp,
    Bocha,
    Querit,
    Parallel,
    #[serde(rename = "jina-search")]
    JinaSearch,
    Firecrawl,
    #[serde(rename = "querit-fetch")]
    QueritFetch,
    #[serde(rename = "firecrawl-fetch")]
    FirecrawlFetch,
}
impl Provider {
    pub(super) fn id(self) -> &'static str {
        match self {
            Self::Tavily => "tavily",
            Self::Brave => "brave",
            Self::Exa => "exa",
            Self::Direct => "direct",
            Self::Jina => "jina",
            Self::Zhipu => "zhipu",
            Self::Searxng => "searxng",
            Self::ExaMcp => "exa-mcp",
            Self::Bocha => "bocha",
            Self::Querit => "querit",
            Self::Parallel => "parallel",
            Self::JinaSearch => "jina-search",
            Self::Firecrawl => "firecrawl",
            Self::QueritFetch => "querit-fetch",
            Self::FirecrawlFetch => "firecrawl-fetch",
        }
    }
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Tavily => "Tavily",
            Self::Brave => "Brave Search",
            Self::Exa => "Exa",
            Self::Direct => "直接获取",
            Self::Jina | Self::JinaSearch => "Jina",
            Self::Zhipu => "智谱 Zhipu",
            Self::Searxng => "SearXNG",
            Self::ExaMcp => "Exa MCP",
            Self::Bocha => "博查 Bocha",
            Self::Querit | Self::QueritFetch => "Querit",
            Self::Parallel => "Parallel",
            Self::Firecrawl | Self::FirecrawlFetch => "Firecrawl",
        }
    }
    pub(super) fn endpoint(self) -> &'static str {
        match self {
            Self::Tavily => "https://api.tavily.com",
            Self::Brave => "https://api.search.brave.com",
            Self::Exa => "https://api.exa.ai",
            Self::Direct => "",
            Self::Jina => "https://r.jina.ai",
            Self::Zhipu => "https://open.bigmodel.cn/api/paas/v4/web_search",
            Self::Searxng => "http://localhost:8080",
            Self::ExaMcp => "https://mcp.exa.ai/mcp",
            Self::Bocha => "https://api.bochaai.com",
            Self::Querit | Self::QueritFetch => "https://api.querit.ai",
            Self::Parallel => "https://api.parallel.ai",
            Self::JinaSearch => "https://s.jina.ai",
            Self::Firecrawl | Self::FirecrawlFetch => "https://api.firecrawl.dev",
        }
    }
    pub(super) fn website(self) -> &'static str {
        match self {
            Self::Tavily => "https://tavily.com",
            Self::Brave => "https://brave.com/search/api/",
            Self::Exa | Self::ExaMcp => "https://exa.ai",
            Self::Direct => "https://developer.mozilla.org/zh-CN/docs/Web/HTTP",
            Self::Jina | Self::JinaSearch => "https://jina.ai/reader",
            Self::Zhipu => "https://docs.bigmodel.cn/cn/guide/tools/web-search",
            Self::Searxng => "https://docs.searxng.org",
            Self::Bocha => "https://bochaai.com",
            Self::Querit | Self::QueritFetch => "https://querit.ai",
            Self::Parallel => "https://parallel.ai",
            Self::Firecrawl | Self::FirecrawlFetch => "https://firecrawl.dev",
        }
    }
    pub(super) fn key_site(self) -> Option<&'static str> {
        Some(match self {
            Self::Direct | Self::Searxng => return None,
            Self::Tavily => "https://app.tavily.com/home",
            Self::Brave => "https://api-dashboard.search.brave.com/app/keys",
            Self::Exa | Self::ExaMcp => "https://dashboard.exa.ai/api-keys",
            Self::Jina | Self::JinaSearch => "https://jina.ai",
            Self::Zhipu => "https://bigmodel.cn",
            Self::Bocha => "https://open.bochaai.com/overview",
            Self::Querit | Self::QueritFetch => "https://www.querit.ai/en/dashboard/api-keys",
            Self::Parallel => "https://platform.parallel.ai",
            Self::Firecrawl | Self::FirecrawlFetch => "https://firecrawl.dev/app/api-keys",
        })
    }
    pub(super) fn description(self) -> &'static str {
        match self {
            Self::Tavily => "面向 AI 应用的网页搜索，返回相关来源与正文摘要。",
            Self::Brave => "独立搜索索引；单次最多返回 20 条结果。",
            Self::Exa => "面向 AI 的神经搜索 API，支持语义检索高质量网页。",
            Self::Direct => "直接读取静态网页，无需密钥，不运行脚本。",
            Self::Jina | Self::JinaSearch => "Jina Reader 搜索与阅读接口，用于检索并提取网页正文。",
            Self::Zhipu => "GLM 联网搜索服务。",
            Self::Searxng => "自托管聚合搜索，可按需配置 Basic 认证。",
            Self::ExaMcp => "通过 MCP 搜索网页；匿名调用受供应商限额约束。",
            Self::Bocha => "中文网页搜索与内容摘要。",
            Self::Querit | Self::QueritFetch => "网页搜索与正文提取服务。",
            Self::Parallel => "网页搜索与相关内容片段。",
            Self::Firecrawl | Self::FirecrawlFetch => "网页搜索与 Markdown 正文提取。",
        }
    }
    pub(super) fn is_fetch(self) -> bool {
        matches!(
            self,
            Self::Direct | Self::Jina | Self::QueritFetch | Self::FirecrawlFetch
        )
    }
    pub(super) fn requires_key(self) -> bool {
        !matches!(
            self,
            Self::Direct
                | Self::Jina
                | Self::Searxng
                | Self::ExaMcp
                | Self::Firecrawl
                | Self::FirecrawlFetch
        )
    }
}
