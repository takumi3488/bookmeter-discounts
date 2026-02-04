#!/bin/bash
curl -m 30 "https://www.amazon.co.jp/dp/$1" \
  -H 'accept-language: ja,en;q=0.9,en-GB;q=0.8,en-US;q=0.7' \
  -H 'user-agent: Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0'
