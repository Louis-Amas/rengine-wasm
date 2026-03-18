# To Improve

## Lock

- Currently is_waiting flag is locking the full exchange when we are waiting for a response to a post request (opening an order for example). We could improve this to lock only on a specific book
