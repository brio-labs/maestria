from wishlist.items import Item, make_item


def test_make_item():
    item = make_item("x", 2)
    assert item.price == 2
