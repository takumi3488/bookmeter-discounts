create table if not exists public.books (
    bookmeter_id bigint not null,
    amazon_url text not null,
    kindle_id text,
    title text not null,
    basis_price integer,
    price integer,
    discount_rate real,
    updated_at timestamp not null,
    active_at timestamp,
    is_kindle_unlimited boolean not null default false,
    binding_name text,
    constraint books_pkey primary key (bookmeter_id)
);
create index if not exists books_kindle_id_index on public.books (kindle_id);
create index if not exists books_basis_price_index on public.books (basis_price);
create index if not exists books_price_index on public.books (price);
create index if not exists books_discount_rate_index on public.books (discount_rate);
create index if not exists books_title_index on public.books (title);

-- 割引中またはKindle Unlimited対象の本のビュー (外部サービスから参照される)
create or replace view public.books_discounted as
SELECT bookmeter_id,
    amazon_url,
    kindle_id,
    title,
    basis_price,
    price,
    discount_rate,
    updated_at,
    active_at,
    is_kindle_unlimited
   FROM books
  WHERE discount_rate IS NOT NULL AND discount_rate >= 0.15::double precision OR is_kindle_unlimited;

-- 中古本サイト (bookoff / valuebooks / netoff) の商品オファー
create table if not exists public.used_book_offers (
    bookmeter_id bigint not null,
    site text not null,
    product_id text,
    product_url text,
    price integer,
    condition text,
    in_stock boolean not null default false,
    updated_at timestamp not null,
    constraint used_book_offers_pkey primary key (bookmeter_id, site),
    constraint used_book_offers_bookmeter_id_fkey foreign key (bookmeter_id) references public.books (bookmeter_id) on delete cascade
);
create index if not exists used_book_offers_product_id_index on public.used_book_offers (product_id);
