//! UI-thread poll loop: a 25 ms timer runs every registered ticker; a ticker returning
//! false is dropped. Workers never touch widgets, the UI pulls from their channels here.

use std::cell::RefCell;

type Ticker = Box<dyn FnMut() -> bool>;

thread_local! {
    static TICKERS: RefCell<Vec<Ticker>> = RefCell::new(vec![]);
}

pub fn every_tick(f: impl FnMut() -> bool + 'static) {
    TICKERS.with(|t| t.borrow_mut().push(Box::new(f)));
}

pub fn tick() {
    // Take the list so tickers may register new tickers while running.
    let mut cur = TICKERS.with(|t| std::mem::take(&mut *t.borrow_mut()));
    cur.retain_mut(|f| f());
    TICKERS.with(|t| {
        let mut t = t.borrow_mut();
        let added = std::mem::take(&mut *t);
        *t = cur;
        t.extend(added);
    });
}
