"""Wishlist package."""

from wishlist.items import Item


def create_default():
    return Item("default", 1)
