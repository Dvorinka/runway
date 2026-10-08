//! Speed insights — real-user monitoring beacon.
//!
//! The dashboard stores `config.speed_insights.enabled`; the worker then
//! injects [`beacon_snippet`] into every deployed HTML page. The script
//! posts Core Web Vitals to `/_runway-rum` on the same origin — Traefik
//! routes that path to the API (see `traefik::update_project_config`),
//! so no CORS and no dependence on the dashboard hostname being public.

/// Inline script injected into deployed HTML. Fires a `pageview`
/// beacon on load and on SPA navigations (pushState/replaceState/
/// popstate), exposes `window.rw.event(name)` for custom events, and
/// posts Core Web Vitals once per visibility-hide (re-armed when the
/// page becomes visible again). The server decides which kinds to
/// store based on the project's enabled flags.
pub fn beacon_snippet() -> &'static str {
    concat!(
        "<script>(function(){try{",
        "var m={},sent=false;",
        "function post(o){",
        "var b=new Blob([JSON.stringify(o)],{type:'application/json'});",
        "navigator.sendBeacon('/_runway-rum',b)}",
        "function pv(){post({t:'pv',p:location.pathname,r:document.referrer||''})}",
        "window.rw={event:function(n){",
        "post({t:'ev',n:String(n).slice(0,64),p:location.pathname})}};",
        "var P=history.pushState,R=history.replaceState;",
        "function nav(){setTimeout(pv,0)}",
        "if(P)history.pushState=function(){",
        "var r=P.apply(this,arguments);nav();return r};",
        "if(R)history.replaceState=function(){",
        "var r=R.apply(this,arguments);nav();return r};",
        "addEventListener('popstate',nav);pv();",
        "function send(){",
        "if(sent)return;sent=true;",
        "m.p=location.pathname;",
        "var n=performance.getEntriesByType('navigation')[0];",
        "if(n)m.ttfb=Math.round(n.responseStart);",
        "if(m.cls)m.cls=Math.round(m.cls*1000)/1000;",
        "if(!m.lcp&&!m.fcp)return;",
        "post(m)}",
        "try{new PerformanceObserver(function(l){",
        "var e=l.getEntries();m.lcp=Math.round(e[e.length-1].startTime)}",
        ").observe({type:'largest-contentful-paint',buffered:true})}catch(_){}",
        "try{new PerformanceObserver(function(l){",
        "var e=l.getEntries();for(var i=0;i<e.length;i++)",
        "if(e[i].name==='first-contentful-paint')m.fcp=Math.round(e[i].startTime)}",
        ").observe({type:'paint',buffered:true})}catch(_){}",
        "try{new PerformanceObserver(function(l){",
        "var e=l.getEntries();for(var i=0;i<e.length;i++)",
        "if(!e[i].hadRecentInput)m.cls=(m.cls||0)+e[i].value}",
        ").observe({type:'layout-shift',buffered:true})}catch(_){}",
        "try{new PerformanceObserver(function(l){",
        "var e=l.getEntries();for(var i=0;i<e.length;i++)",
        "var d=Math.round(e[i].duration);if(d>(m.inp||0))m.inp=d}",
        ").observe({type:'event',durationThreshold:16,buffered:true})}catch(_){}",
        "document.addEventListener('visibilitychange',function(){",
        "if(document.visibilityState==='hidden')send();else sent=false});",
        "addEventListener('pagehide',send)",
        "}catch(e){}})();</script>"
    )
}

/// Clamp a metric to a plausible range; returns None for junk input.
/// 10 minutes covers pathological pages while rejecting `1e300` noise.
pub fn clamp_metric(v: f64) -> Option<f64> {
    if v.is_finite() && (0.0..600_000.0).contains(&v) {
        Some(v)
    } else {
        None
    }
}

pub fn sanitize_path(s: &str) -> String {
    let mut p: String = s.chars().take(256).collect();
    if !p.starts_with('/') {
        p.insert(0, '/');
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_is_self_contained() {
        let s = beacon_snippet();
        assert!(s.starts_with("<script>"));
        assert!(s.ends_with("</script>"));
        assert!(s.contains("/_runway-rum"));
        assert!(s.contains("sendBeacon"));
    }

    #[test]
    fn clamps_metrics() {
        assert_eq!(clamp_metric(123.4), Some(123.4));
        assert_eq!(clamp_metric(-1.0), None);
        assert_eq!(clamp_metric(f64::INFINITY), None);
        assert_eq!(clamp_metric(f64::NAN), None);
        assert_eq!(clamp_metric(700_000.0), None);
    }

    #[test]
    fn sanitizes_paths() {
        assert_eq!(sanitize_path("about"), "/about");
        assert_eq!(sanitize_path("/x"), "/x");
        assert_eq!(sanitize_path(&"/".repeat(300)).len(), 256);
    }
}
