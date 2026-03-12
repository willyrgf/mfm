# Nginx configuration templates with production-hardened defaults
{
  pkgs,
  package ? pkgs.nginx,
}:

let
  nginx = package;

  nginxConfTemplate = pkgs.writeText "nginx.conf.template" ''
    worker_processes auto;
    error_log NGINX_DIR/logs/error.log warn;
    pid NGINX_DIR/run/nginx.pid;

    events {
        worker_connections 1024;
        multi_accept on;
    }

    http {
        include ${nginx}/conf/mime.types;
        default_type application/octet-stream;

        access_log NGINX_DIR/logs/access.log;

        sendfile on;
        tcp_nopush on;
        tcp_nodelay on;
        keepalive_timeout 65;

        server_tokens off;

        # Gzip compression
        gzip on;
        gzip_vary on;
        gzip_proxied any;
        gzip_comp_level 6;
        gzip_types text/plain text/css text/xml application/json application/javascript application/xml+rss application/atom+xml image/svg+xml;

        server {
            listen HTTP_PORT default_server;
            listen HTTPS_PORT ssl default_server;
            http2 on;
            server_name _;

            ssl_certificate NGINX_DIR/ssl/live/localhost/fullchain.pem;
            ssl_certificate_key NGINX_DIR/ssl/live/localhost/privkey.pem;

            root NGINX_DIR/html;

            location / {
                return 444;
            }
        }

        include NGINX_DIR/conf/sites-enabled/*.conf;
    }
  '';

  siteProxyTemplate = pkgs.writeText "site-proxy.conf.template" ''
    server {
        listen HTTP_PORT;
        server_name SITE_DOMAIN;

        # ACME challenge location for Let's Encrypt
        location /.well-known/acme-challenge/ {
            root NGINX_DIR/html;
        }

        location / {
            return 301 https://$server_name$request_uri;
        }
    }

    server {
        listen HTTPS_PORT ssl;
        http2 on;
        server_name SITE_DOMAIN;

        ssl_certificate NGINX_DIR/ssl/live/SITE_DOMAIN/fullchain.pem;
        ssl_certificate_key NGINX_DIR/ssl/live/SITE_DOMAIN/privkey.pem;

        # Security headers
        add_header X-Frame-Options "SAMEORIGIN" always;
        add_header X-Content-Type-Options "nosniff" always;
        add_header X-XSS-Protection "1; mode=block" always;
        add_header Referrer-Policy "strict-origin-when-cross-origin" always;
        add_header Strict-Transport-Security "max-age=31536000; includeSubDomains" always;

        # Proxy settings with fast-failure timeout
        proxy_connect_timeout 5s;
        proxy_send_timeout 60s;
        proxy_read_timeout 60s;

        location / {
            proxy_pass http://UPSTREAM_HOST:UPSTREAM_PORT;
            proxy_http_version 1.1;
            proxy_set_header Upgrade $http_upgrade;
            proxy_set_header Connection "upgrade";
            proxy_set_header Host $host;
            proxy_set_header X-Real-IP $remote_addr;
            proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
            proxy_set_header X-Forwarded-Proto $scheme;
        }
    }
  '';

  siteStaticTemplate = pkgs.writeText "site-static.conf.template" ''
    server {
        listen HTTP_PORT;
        server_name SITE_DOMAIN;

        location /.well-known/acme-challenge/ {
            root NGINX_DIR/html;
        }

        location / {
            return 301 https://$server_name$request_uri;
        }
    }

    server {
        listen HTTPS_PORT ssl;
        http2 on;
        server_name SITE_DOMAIN;

        ssl_certificate NGINX_DIR/ssl/live/SITE_DOMAIN/fullchain.pem;
        ssl_certificate_key NGINX_DIR/ssl/live/SITE_DOMAIN/privkey.pem;

        # Security headers
        add_header X-Frame-Options "SAMEORIGIN" always;
        add_header X-Content-Type-Options "nosniff" always;
        add_header X-XSS-Protection "1; mode=block" always;
        add_header Referrer-Policy "strict-origin-when-cross-origin" always;
        add_header Strict-Transport-Security "max-age=31536000; includeSubDomains" always;

        root SITE_ROOT;
        index index.html index.htm;

        location / {
            try_files $uri $uri/ =404;
        }
    }
  '';

in
{
  inherit
    nginx
    nginxConfTemplate
    siteProxyTemplate
    siteStaticTemplate
    ;
}
