// Events as on Ethereum: topic 0 is keccak256 of the signature, indexed
// fields are topics, the rest is ABI-encoded data.
contract events {
	event Plain(uint256 a, bool b);
	event Indexed(address indexed from, address indexed to, uint256 value);
	event Mixed(int8 indexed s, bytes4 indexed t, uint64 x, address y);
	event Anon(uint256 indexed a, uint256 b) anonymous;
	event Empty();

	uint256 public fired;

	function fire() public {
		emit Plain(1, true);
		emit Indexed(msg.sender, address(0xb0b), 5);
		emit Mixed(-2, 0xcafebabe, 7, address(this));
		emit Anon(9, 10);
		emit Empty();
		fired += 1;
	}

	function fireThenRevert() public {
		emit Plain(1, true);
		fired += 1;
		revert();
	}
}
